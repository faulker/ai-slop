/*
aetr COFDM modem shim.

Implements the COFDMTV burst format (Schmidl-Cox sync symbol, BCH(255,71)
preamble decoded with ordered-statistics decoding, four QPSK payload symbols
carrying a CA-SCL polar code of length 2048) on top of the vendored aicodix
header libraries. Protocol design by Ahmet Inan (aicodix, zero-clause BSD);
this file is an independent implementation for a fixed 48 kHz mono f32
configuration. See core/cpp/aicodix/VENDORED.md.

Fixed layout at 48000 Hz:
  symbol_length 7680 samples (160 ms), guard 960, extended 8640 (180 ms)
  256 payload carriers at 6.25 Hz spacing = 1600 Hz, centered at 1500 Hz
  burst = sync + preamble + 4 payload symbols + fade-out = 60480 samples
*/

#include <algorithm>
#include <iostream>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <cassert>
#include <new>

namespace DSP { using std::abs; using std::min; using std::cos; using std::sin; }

#include "complex.hh"
#include "const.hh"
#include "fft.hh"
#include "mls.hh"
#include "crc.hh"
#include "psk.hh"
#include "bitman.hh"
#include "xorshift.hh"
#include "simd.hh"
#include "polar_helper.hh"
#include "polar_encoder.hh"
#include "polar_list_decoder.hh"
#include "osd.hh"
#include "bose_chaudhuri_hocquenghem_encoder.hh"
#include "schmidl_cox.hh"
#include "bip_buffer.hh"
#include "theil_sen.hh"
#include "blockdc.hh"
#include "filter.hh"
#include "delay.hh"
#include "hilbert.hh"
#include "phasor.hh"

#include "cofdm_tables.hh"
#include "shim.hh"

namespace {

typedef float value;
typedef DSP::Complex<value> cmplx;
typedef DSP::Const<value> Const;

// Fixed modem geometry at 48 kHz.
const int RATE = 48000;
const int code_order = 11;
const int mod_bits = 2;
const int code_len = 1 << code_order;
const int symbol_count = 4;
const int symbol_length = (1280 * RATE) / 8000;
const int guard_length = symbol_length / 8;
const int extended_length = symbol_length + guard_length;
const int max_bits = 1360;
const int cor_seq_len = 127;
const int cor_seq_off = 1 - cor_seq_len;
const int cor_seq_poly = 0b10001001;
const int pre_seq_len = 255;
const int pre_seq_off = -pre_seq_len / 2;
const int pre_seq_poly = 0b100101011;
const int pay_car_cnt = 256;
const int pay_car_off = -pay_car_cnt / 2;
// Center of the OFDM band in Hz; must sit inside the FM voice passband.
const int carrier_frequency = 1500;
// Emitted burst: sync + preamble + 4 payload symbols + one fade-out symbol.
const int burst_symbols = 2 + symbol_count + 1;
const int burst_samples = burst_symbols * extended_length;
// 55-bit metadata word: (station magic << 8) | operation mode. aetr has no
// call signs, so a fixed magic marks bursts as ours.
const uint64_t META_MAGIC = 0x41455452; // "AETR"

// BCH(255,71) generator minimal polynomials shared by preamble encode/decode.
#define AETR_BCH_POLYS \
	0b100011101, 0b101110111, 0b111110011, 0b101101001, \
	0b110111101, 0b111100111, 0b100101011, 0b111010111, \
	0b000010011, 0b101100101, 0b110001011, 0b101100011, \
	0b100011011, 0b100111111, 0b110001101, 0b100101101, \
	0b101011111, 0b111111001, 0b111000011, 0b100111001, \
	0b110101001, 0b000011111, 0b110000111, 0b110110001

/// Maps an operation mode (14/15/16) to its polar data-bit count and frozen
/// table. Returns false for unknown modes.
bool mode_params(int oper_mode, int *data_bits, const uint32_t **frozen)
{
	switch (oper_mode) {
	case 14: *data_bits = 1360; *frozen = frozen_2048_1392; return true;
	case 15: *data_bits = 1024; *frozen = frozen_2048_1056; return true;
	case 16: *data_bits = 680;  *frozen = frozen_2048_712;  return true;
	}
	return false;
}

/// Systematic CA-SCL polar encoding: appends a CRC32 to the payload bits and
/// produces the length-2048 NRZ codeword.
class CofdmPolarEncoder {
	CODE::CRC<uint32_t> crc;
	CODE::PolarSysEnc<int8_t> sysenc;
	int8_t mesg[max_bits + 32];

	static int nrz(bool bit) { return 1 - 2 * bit; }

public:
	CofdmPolarEncoder() : crc(0x8F6E37A0) {}

	/// Encodes data_bits worth of message bytes (little-endian bit order)
	/// plus CRC32 into the codeword.
	void operator()(int8_t *code, const uint8_t *message, const uint32_t *frozen, int data_bits)
	{
		for (int i = 0; i < data_bits; ++i)
			mesg[i] = nrz(CODE::get_le_bit(message, i));
		crc.reset();
		for (int i = 0; i < data_bits / 8; ++i)
			crc(message[i]);
		for (int i = 0; i < 32; ++i)
			mesg[i + data_bits] = nrz((crc() >> i) & 1);
		sysenc(code, mesg, frozen, code_order);
	}
};

/// CRC-aided successive-cancellation list decoding of the payload codeword.
/// Returns the number of bit flips relative to the hard decisions on success,
/// or a negative value when no list entry passes the CRC.
class CofdmPolarDecoder {
#ifdef __AVX2__
	typedef SIMD<int8_t, 32> mesg_type;
#else
	typedef SIMD<int8_t, 16> mesg_type;
#endif
	CODE::CRC<uint32_t> crc;
	CODE::PolarEncoder<mesg_type> reencode;
	CODE::PolarListDecoder<mesg_type, code_order> decode;
	mesg_type mesg[max_bits + 32], mess[code_len];

	/// Recovers the systematic bit view of every list path by re-encoding the
	/// decoded message and reading back the non-frozen codeword positions.
	void systematic(const uint32_t *frozen, int crc_bits)
	{
		reencode(mess, mesg, frozen, code_order);
		for (int i = 0, j = 0; i < code_len && j < crc_bits; ++i)
			if (!((frozen[i / 32] >> (i % 32)) & 1))
				mesg[j++] = mess[i];
	}

public:
	CofdmPolarDecoder() : crc(0x8F6E37A0) {}

	int operator()(uint8_t *message, const int8_t *code, const uint32_t *frozen, int data_bits)
	{
		int crc_bits = data_bits + 32;
		decode(nullptr, mesg, code, frozen, code_order);
		systematic(frozen, crc_bits);
		int best = -1;
		for (int k = 0; k < mesg_type::SIZE; ++k) {
			crc.reset();
			for (int i = 0; i < crc_bits; ++i)
				crc(mesg[i].v[k] < 0);
			if (crc() == 0) {
				best = k;
				break;
			}
		}
		if (best < 0)
			return -1;
		int flips = 0;
		for (int i = 0, j = 0; i < data_bits; ++i, ++j) {
			while ((frozen[j / 32] >> (j % 32)) & 1)
				++j;
			bool received = code[j] < 0;
			bool decoded = mesg[i].v[best] < 0;
			flips += received != decoded;
			CODE::set_le_bit(message, i, decoded);
		}
		return flips;
	}
};

/// One-shot burst encoder. All state is reset per encode() call.
class Encoder {
	DSP::FastFourierTransform<symbol_length, cmplx, -1> fwd;
	DSP::FastFourierTransform<symbol_length, cmplx, 1> bwd;
	CODE::CRC<uint16_t> crc16;
	CODE::BoseChaudhuriHocquenghemEncoder<255, 71> bch;
	CofdmPolarEncoder polar;
	cmplx temp[symbol_length], freq[symbol_length], papr_time[symbol_length];
	cmplx prev[pay_car_cnt], guard[guard_length];
	bool papr_used[symbol_length];
	int8_t code[code_len];
	uint8_t mesg[max_bits / 8];
	int carrier_offset;

	static int nrz(bool bit) { return 1 - 2 * bit; }

	static cmplx mod_map(const int8_t *b)
	{
		return PhaseShiftKeying<4, cmplx, int8_t>::map(const_cast<int8_t *>(b));
	}

	int bin(int carrier) const
	{
		return (carrier + carrier_offset + symbol_length) % symbol_length;
	}

	/// Tames peak-to-average power ratio: soft-clips the time-domain symbol
	/// and restores the active-carrier mask in the frequency domain.
	void improve_papr()
	{
		for (int i = 0; i < symbol_length; ++i)
			papr_used[i] = freq[i].real() != 0 || freq[i].imag() != 0;
		bwd(papr_time, freq);
		value factor = 1 / std::sqrt(value(symbol_length));
		for (int i = 0; i < symbol_length; ++i)
			papr_time[i] *= factor;
		for (int i = 0; i < symbol_length; ++i) {
			value pwr = norm(papr_time[i]);
			if (pwr > value(1))
				papr_time[i] /= std::sqrt(pwr);
		}
		fwd(freq, papr_time);
		for (int i = 0; i < symbol_length; ++i) {
			if (papr_used[i])
				freq[i] *= factor;
			else
				freq[i] = 0;
		}
	}

	/// Converts the assembled frequency-domain symbol into normalized
	/// time-domain samples in temp[].
	void transform()
	{
		improve_papr();
		bwd(temp, freq);
		for (int i = 0; i < symbol_length; ++i)
			temp[i] /= std::sqrt(value(8 * symbol_length));
	}

	/// Emits one extended symbol: raised-cosine crossfade from the previous
	/// symbol's head into this symbol's cyclic prefix, then the symbol body.
	void emit(float *out, int *pos, bool data_symbol)
	{
		for (int i = 0; i < guard_length; ++i) {
			value x = value(i) / value(guard_length - 1);
			value ratio(0.5);
			if (data_symbol)
				x = std::min(x, ratio) / ratio;
			value y = value(0.5) * (1 - std::cos(Const::Pi() * x));
			cmplx sum = DSP::lerp(guard[i], temp[i + symbol_length - guard_length], y);
			out[(*pos)++] = sum.real();
		}
		for (int i = 0; i < guard_length; ++i)
			guard[i] = temp[i];
		for (int i = 0; i < symbol_length; ++i)
			out[(*pos)++] = temp[i].real();
	}

	/// Builds the Schmidl-Cox synchronization symbol: a differentially
	/// encoded MLS on every other carrier, giving half-symbol periodicity.
	void sync_symbol()
	{
		CODE::MLS seq(cor_seq_poly);
		value factor = std::sqrt(value(2 * symbol_length) / value(cor_seq_len));
		for (int i = 0; i < symbol_length; ++i)
			freq[i] = 0;
		freq[bin(cor_seq_off - 2)] = factor;
		for (int i = 0; i < cor_seq_len; ++i)
			freq[bin(2 * i + cor_seq_off)] = nrz(seq());
		for (int i = 0; i < cor_seq_len; ++i)
			freq[bin(2 * i + cor_seq_off)] *= freq[bin(2 * (i - 1) + cor_seq_off)];
		transform();
	}

	/// Builds the preamble symbol: 55-bit metadata + CRC16, BCH(255,71)
	/// parity, differential BPSK along carriers whitened by an MLS. Also
	/// seeds prev[] as the phase reference for the first payload symbol.
	void preamble_symbol(uint64_t md)
	{
		uint8_t data[9] = {0}, parity[23] = {0};
		for (int i = 0; i < 55; ++i)
			CODE::set_be_bit(data, i, (md >> i) & 1);
		crc16.reset();
		uint16_t cs = crc16(md << 9);
		for (int i = 0; i < 16; ++i)
			CODE::set_be_bit(data, i + 55, (cs >> i) & 1);
		bch(data, parity);
		CODE::MLS seq(pre_seq_poly);
		value factor = std::sqrt(value(symbol_length) / value(pre_seq_len));
		for (int i = 0; i < symbol_length; ++i)
			freq[i] = 0;
		freq[bin(pre_seq_off - 1)] = factor;
		for (int i = 0; i < 71; ++i)
			freq[bin(i + pre_seq_off)] = nrz(CODE::get_be_bit(data, i));
		for (int i = 71; i < pre_seq_len; ++i)
			freq[bin(i + pre_seq_off)] = nrz(CODE::get_be_bit(parity, i - 71));
		for (int i = 0; i < pre_seq_len; ++i)
			freq[bin(i + pre_seq_off)] *= freq[bin(i - 1 + pre_seq_off)];
		for (int i = 0; i < pre_seq_len; ++i)
			freq[bin(i + pre_seq_off)] *= nrz(seq());
		for (int i = 0; i < pay_car_cnt; ++i)
			prev[i] = freq[bin(i + pay_car_off)];
		transform();
	}

	/// Builds one QPSK payload symbol, differentially encoded per carrier
	/// against the previous symbol.
	void payload_symbol(int symbol_number)
	{
		for (int i = 0; i < symbol_length; ++i)
			freq[i] = 0;
		for (int i = 0; i < pay_car_cnt; ++i)
			freq[bin(i + pay_car_off)] = prev[i] *= mod_map(code + mod_bits * (pay_car_cnt * symbol_number + i));
		transform();
	}

public:
	Encoder() : crc16(0xA8F4), bch({AETR_BCH_POLYS})
	{
		carrier_offset = (carrier_frequency * symbol_length) / RATE;
	}

	/// Encodes one payload into a full burst of burst_samples f32 samples.
	/// payload must hold exactly data_bits/8 bytes for the operation mode.
	int encode(int oper_mode, const uint8_t *payload, float *out)
	{
		int data_bits;
		const uint32_t *frozen;
		if (!mode_params(oper_mode, &data_bits, &frozen))
			return -1;
		CODE::Xorshift32 scrambler;
		for (int i = 0; i < data_bits / 8; ++i)
			mesg[i] = payload[i] ^ scrambler();
		polar(code, mesg, frozen, data_bits);
		for (int i = 0; i < guard_length; ++i)
			guard[i] = 0;
		int pos = 0;
		sync_symbol();
		emit(out, &pos, true);
		preamble_symbol((META_MAGIC << 8) | uint64_t(oper_mode));
		emit(out, &pos, true);
		for (int j = 0; j < symbol_count; ++j) {
			payload_symbol(j);
			emit(out, &pos, true);
		}
		// Fade-out: crossfade the last symbol's head down into silence.
		for (int i = 0; i < symbol_length; ++i)
			temp[i] = 0;
		emit(out, &pos, false);
		return pos;
	}
};

/// Streaming burst decoder: Schmidl-Cox acquisition, OSD preamble decode,
/// per-block payload demodulation with Theil-Sen phase tracking.
class Decoder {
	static const int filter_length = (((33 * RATE) / 8000) & ~3) | 1;
	static const int buffer_length = 4 * extended_length;
	static const int search_position = extended_length;

	DSP::FastFourierTransform<symbol_length, cmplx, -1> fwd;
	SchmidlCox<value, cmplx, search_position, symbol_length / 2, guard_length> correlator;
	DSP::BlockDC<value, value> block_dc;
	DSP::Hilbert<cmplx, filter_length> hilbert;
	DSP::BipBuffer<cmplx, buffer_length> buffer;
	DSP::TheilSenEstimator<value, pay_car_cnt> tse;
	DSP::Phasor<cmplx> osc;
	CODE::CRC<uint16_t> crc16;
	CODE::OrderedStatisticsDecoder<255, 71, 2> osd;
	CofdmPolarDecoder polar;
	cmplx temp[extended_length], freq[symbol_length];
	cmplx prev[pay_car_cnt], cons[pay_car_cnt];
	cmplx cor_seq_buf[symbol_length / 2];
	value index[pay_car_cnt], phase[pay_car_cnt];
	int8_t generator[255 * 71];
	int8_t soft[pre_seq_len];
	uint8_t hard[(pre_seq_len + 7) / 8];
	int8_t code[code_len];
	int symbol_number = symbol_count;
	int symbol_position = search_position + extended_length;
	int stored_position = 0;
	int staged_position = 0;
	int staged_mode = 0;
	int operation_mode = 0;
	int accumulated = 0;
	value stored_cfo_rad = 0;
	value staged_cfo_rad = 0;
	bool stored_check = false;
	bool staged_check = false;
	const cmplx *buf;

	static int nrz(bool bit) { return 1 - 2 * bit; }

	static int bin(int carrier)
	{
		return (carrier + symbol_length) % symbol_length;
	}

	static cmplx mod_map(const int8_t *b)
	{
		return PhaseShiftKeying<4, cmplx, int8_t>::map(const_cast<int8_t *>(b));
	}

	static void mod_hard(int8_t *b, cmplx c)
	{
		PhaseShiftKeying<4, cmplx, int8_t>::hard(b, c);
	}

	static void mod_soft(int8_t *b, cmplx c, value precision)
	{
		PhaseShiftKeying<4, cmplx, int8_t>::soft(b, c, precision);
	}

	/// Differential demodulation with erasure of unreliable carriers.
	static cmplx demod_or_erase(cmplx curr, cmplx prv)
	{
		if (norm(prv) <= 0)
			return 0;
		cmplx cons = curr / prv;
		if (norm(cons) > 4)
			return 0;
		return cons;
	}

	/// Builds the half-length reference spectrum the Schmidl-Cox correlator
	/// matches against.
	const cmplx *cor_seq()
	{
		CODE::MLS seq(cor_seq_poly);
		for (int i = 0; i < symbol_length / 2; ++i)
			cor_seq_buf[i] = 0;
		for (int i = 0; i < cor_seq_len; ++i)
			cor_seq_buf[(i + cor_seq_off / 2 + symbol_length / 2) % (symbol_length / 2)] = nrz(seq());
		return cor_seq_buf;
	}

	/// DC-blocks a real sample and converts it to its analytic signal.
	cmplx convert(float sample)
	{
		return hilbert(block_dc(sample));
	}

	/// Estimates and removes the linear phase slope across payload carriers
	/// (residual timing/frequency offset) via a Theil-Sen fit.
	void compensate()
	{
		int count = 0;
		for (int i = 0; i < pay_car_cnt; ++i) {
			cmplx con = cons[i];
			if (con.real() != 0 && con.imag() != 0) {
				int8_t tmp[mod_bits];
				mod_hard(tmp, con);
				index[count] = i + pay_car_off;
				phase[count] = arg(con * conj(mod_map(tmp)));
				++count;
			}
		}
		tse.compute(index, phase, count);
		for (int i = 0; i < pay_car_cnt; ++i)
			cons[i] *= DSP::polar<value>(1, -tse(i + pay_car_off));
	}

	/// Signal-to-error power ratio of the demodulated constellation, used to
	/// scale soft bits.
	value precision()
	{
		value sp = 0, np = 0;
		for (int i = 0; i < pay_car_cnt; ++i) {
			int8_t tmp[mod_bits];
			mod_hard(tmp, cons[i]);
			cmplx h = mod_map(tmp);
			cmplx error = cons[i] - h;
			sp += norm(h);
			np += norm(error);
		}
		return sp / np;
	}

	/// Soft-demaps the current symbol's carriers into the codeword buffer.
	void demap()
	{
		value pre = precision();
		for (int i = 0; i < pay_car_cnt; ++i)
			mod_soft(code + mod_bits * (symbol_number * pay_car_cnt + i), cons[i], pre);
	}

	/// Decodes the staged preamble candidate: OSD on the BCH(255,71) block,
	/// then CRC16 and magic/mode validation.
	bool preamble()
	{
		DSP::Phasor<cmplx> nco;
		nco.omega(-staged_cfo_rad);
		for (int i = 0; i < symbol_length; ++i)
			temp[i] = buf[staged_position + i] * nco();
		fwd(freq, temp);
		CODE::MLS seq(pre_seq_poly);
		for (int i = 0; i < pre_seq_len; ++i)
			freq[bin(i + pre_seq_off)] *= nrz(seq());
		for (int i = 0; i < pre_seq_len; ++i)
			PhaseShiftKeying<2, cmplx, int8_t>::soft(soft + i,
				demod_or_erase(freq[bin(i + pre_seq_off)], freq[bin(i - 1 + pre_seq_off)]), 32);
		if (!osd(hard, soft, generator))
			return false;
		uint64_t md = 0;
		for (int i = 0; i < 55; ++i)
			md |= uint64_t(CODE::get_be_bit(hard, i)) << i;
		uint16_t cs = 0;
		for (int i = 0; i < 16; ++i)
			cs |= uint16_t(CODE::get_be_bit(hard, i + 55)) << i;
		crc16.reset();
		if (crc16(md << 9) != cs)
			return false;
		staged_mode = md & 255;
		if ((md >> 8) != META_MAGIC)
			return false;
		if (staged_mode < 14 || staged_mode > 16)
			return false;
		return true;
	}

public:
	Decoder() : correlator(cor_seq()), crc16(0xA8F4), buf(nullptr)
	{
		CODE::BoseChaudhuriHocquenghemGenerator<255, 71>::matrix(generator, true, {AETR_BCH_POLYS});
		block_dc.samples(filter_length);
	}

	/// Feeds up to extended_length samples. Returns true when a full block
	/// boundary was crossed, meaning process() should run.
	bool feed(const float *samples, int count)
	{
		assert(count <= extended_length);
		for (int i = 0; i < count; ++i) {
			if (correlator(buffer(convert(samples[i])))) {
				stored_cfo_rad = correlator.cfo_rad;
				stored_position = correlator.symbol_pos + accumulated;
				stored_check = true;
			}
			if (++accumulated == extended_length)
				buf = buffer();
		}
		if (accumulated >= extended_length) {
			accumulated -= extended_length;
			if (stored_check) {
				staged_cfo_rad = stored_cfo_rad;
				staged_position = stored_position;
				staged_check = true;
				stored_check = false;
			}
			return true;
		}
		return false;
	}

	/// Advances the decode state machine by one block. Returns an AETR_RX_*
	/// status describing what happened in this block.
	int process()
	{
		int status = AETR_RX_IDLE;
		if (staged_check) {
			staged_check = false;
			if (preamble()) {
				operation_mode = staged_mode;
				osc.omega(-staged_cfo_rad);
				symbol_position = staged_position;
				symbol_number = -1;
				status = AETR_RX_SYNCED;
			} else {
				status = AETR_RX_FAILED;
			}
		}
		if (symbol_number < symbol_count) {
			for (int i = 0; i < extended_length; ++i)
				temp[i] = buf[symbol_position + i] * osc();
			fwd(freq, temp);
			if (symbol_number >= 0) {
				for (int i = 0; i < pay_car_cnt; ++i)
					cons[i] = demod_or_erase(freq[bin(i + pay_car_off)], prev[i]);
				compensate();
				demap();
			}
			if (++symbol_number == symbol_count)
				status = AETR_RX_READY;
			for (int i = 0; i < pay_car_cnt; ++i)
				prev[i] = freq[bin(i + pay_car_off)];
		}
		return status;
	}

	/// Runs the polar list decoder over the demapped codeword. Returns the
	/// payload byte count on success or a negative value on failure.
	int fetch(uint8_t *payload)
	{
		int data_bits;
		const uint32_t *frozen;
		if (!mode_params(operation_mode, &data_bits, &frozen))
			return -1;
		if (polar(payload, code, frozen, data_bits) < 0)
			return -2;
		CODE::Xorshift32 scrambler;
		for (int i = 0; i < data_bits / 8; ++i)
			payload[i] ^= scrambler();
		for (int i = data_bits / 8; i < 170; ++i)
			payload[i] = 0;
		return data_bits / 8;
	}
};

/// Decoder handle exposed through the C ABI, tracking fetch readiness.
struct RxHandle {
	Decoder *decoder;
	bool ready;
};

/// Maps the public mode index (0/1/2) to the COFDMTV operation mode.
int oper_mode_for(int32_t mode)
{
	switch (mode) {
	case 0: return 16;
	case 1: return 15;
	case 2: return 14;
	}
	return -1;
}

} // namespace

extern "C" {

int32_t aetr_modem_payload_bytes(int32_t mode)
{
	switch (mode) {
	case 0: return 85;
	case 1: return 128;
	case 2: return 170;
	}
	return -1;
}

int32_t aetr_modem_burst_samples(void)
{
	return burst_samples;
}

int32_t aetr_modem_encode(int32_t mode, const uint8_t *payload, int32_t payload_len,
                          float *out_pcm, int32_t out_capacity)
{
	int oper_mode = oper_mode_for(mode);
	if (oper_mode < 0 || payload == nullptr || out_pcm == nullptr)
		return -1;
	if (payload_len != aetr_modem_payload_bytes(mode))
		return -2;
	if (out_capacity < burst_samples)
		return -3;
	try {
		// The encoder is ~1 MB of FFT and buffer state; heap-allocate per
		// call (encode happens at most once per user action).
		Encoder *enc = new (std::nothrow) Encoder();
		if (enc == nullptr)
			return -4;
		int written = enc->encode(oper_mode, payload, out_pcm);
		delete enc;
		return written;
	} catch (...) {
		return -5;
	}
}

void *aetr_modem_rx_new(void)
{
	try {
		Decoder *dec = new (std::nothrow) Decoder();
		if (dec == nullptr)
			return nullptr;
		RxHandle *h = new (std::nothrow) RxHandle{dec, false};
		if (h == nullptr) {
			delete dec;
			return nullptr;
		}
		return h;
	} catch (...) {
		return nullptr;
	}
}

int32_t aetr_modem_rx_feed(void *handle, const float *pcm, int32_t len)
{
	if (handle == nullptr || (pcm == nullptr && len > 0) || len < 0)
		return -1;
	RxHandle *h = static_cast<RxHandle *>(handle);
	int32_t best = AETR_RX_IDLE;
	try {
		int32_t off = 0;
		while (off < len) {
			int32_t chunk = len - off;
			if (chunk > extended_length)
				chunk = extended_length;
			if (h->decoder->feed(pcm + off, chunk)) {
				int status = h->decoder->process();
				if (status == AETR_RX_READY)
					h->ready = true;
				// Priority: READY > SYNCED > FAILED > IDLE.
				if (status == AETR_RX_READY ||
				    (status == AETR_RX_SYNCED && best != AETR_RX_READY) ||
				    (status == AETR_RX_FAILED && best == AETR_RX_IDLE))
					best = status;
			}
			off += chunk;
		}
	} catch (...) {
		return -2;
	}
	return best;
}

int32_t aetr_modem_rx_fetch(void *handle, uint8_t *out_payload)
{
	if (handle == nullptr || out_payload == nullptr)
		return -1;
	RxHandle *h = static_cast<RxHandle *>(handle);
	if (!h->ready)
		return -3;
	h->ready = false;
	try {
		return h->decoder->fetch(out_payload);
	} catch (...) {
		return -2;
	}
}

void aetr_modem_rx_free(void *handle)
{
	if (handle == nullptr)
		return;
	RxHandle *h = static_cast<RxHandle *>(handle);
	delete h->decoder;
	delete h;
}

} // extern "C"
