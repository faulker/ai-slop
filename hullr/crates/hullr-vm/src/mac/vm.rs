//! Building the `VZVirtualMachineConfiguration` for the hullr guest.
//!
//! Devices: Linux boot loader (kernel + initramfs + command line), virtio
//! block (sparse raw `disk.img`), virtio net with NAT, virtio entropy, virtio
//! memory balloon, virtio console serial port to `console.log`, virtio vsock,
//! and virtiofs: one multiple-directory share tagged `hullr-host` plus,
//! when available, the Rosetta share tagged `rosetta`.
//!
//! All Virtualization.framework calls in this module are `unsafe` in the
//! generated bindings only because they are foreign calls; the objects are
//! created and configured on one thread before the VM exists.

use crate::boot::{GuestShare, HOST_SHARE_TAG, ROSETTA_TAG};
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSString, NSURL};
use objc2_virtualization::*;
use std::path::{Path, PathBuf};

/// Everything needed to configure the VM.
pub struct VmSpec {
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub cmdline: String,
    pub cpus: u32,
    pub memory_mib: u64,
    pub disk: PathBuf,
    pub console_log: PathBuf,
    pub shares: Vec<GuestShare>,
    pub rosetta: bool,
}

fn file_url(p: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
}

fn ns_err(what: &str, e: Retained<NSError>) -> String {
    format!("{what}: {}", e.localizedDescription())
}

/// Is Rosetta for Linux usable? Prints a hint when it is merely not
/// installed; the guest then falls back to qemu-user binfmt if present.
pub fn rosetta_available() -> bool {
    // SAFETY: class method without arguments.
    let a = unsafe { VZLinuxRosettaDirectoryShare::availability() };
    if a == VZLinuxRosettaAvailability::Installed {
        true
    } else {
        if a == VZLinuxRosettaAvailability::NotInstalled {
            eprintln!(
                "hullr vm: Rosetta is not installed (run `softwareupdate --install-rosetta`); \
                 linux/amd64 containers will use qemu emulation in the guest"
            );
        }
        false
    }
}

/// Build and validate the configuration.
pub fn build_configuration(spec: &VmSpec) -> Result<Retained<VZVirtualMachineConfiguration>, String> {
    // SAFETY: every call below is a plain Objective-C message send on objects
    // created here, with arguments of the types the framework documents.
    unsafe {
        let config = VZVirtualMachineConfiguration::new();

        let boot = VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &file_url(&spec.kernel));
        boot.setInitialRamdiskURL(Some(&file_url(&spec.initrd)));
        boot.setCommandLine(&NSString::from_str(&spec.cmdline));
        config.setBootLoader(Some(&boot));

        let cpus = (spec.cpus as usize).clamp(
            VZVirtualMachineConfiguration::minimumAllowedCPUCount(),
            VZVirtualMachineConfiguration::maximumAllowedCPUCount(),
        );
        config.setCPUCount(cpus);
        let mem = (spec.memory_mib << 20).clamp(
            VZVirtualMachineConfiguration::minimumAllowedMemorySize(),
            VZVirtualMachineConfiguration::maximumAllowedMemorySize(),
        );
        config.setMemorySize(mem & !((1 << 20) - 1));

        let disk = VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &file_url(&spec.disk),
            false,
        )
        .map_err(|e| ns_err("disk image", e))?;
        let block = VZVirtioBlockDeviceConfiguration::initWithAttachment(VZVirtioBlockDeviceConfiguration::alloc(), &disk);
        config.setStorageDevices(&NSArray::from_retained_slice(&[Retained::into_super(block)]));

        let net = VZVirtioNetworkDeviceConfiguration::new();
        net.setAttachment(Some(&VZNATNetworkDeviceAttachment::new()));
        config.setNetworkDevices(&NSArray::from_retained_slice(&[Retained::into_super(net)]));

        config.setEntropyDevices(&NSArray::from_retained_slice(&[Retained::into_super(
            VZVirtioEntropyDeviceConfiguration::new(),
        )]));
        config.setMemoryBalloonDevices(&NSArray::from_retained_slice(&[Retained::into_super(
            VZVirtioTraditionalMemoryBalloonDeviceConfiguration::new(),
        )]));

        let console = VZFileSerialPortAttachment::initWithURL_append_error(
            VZFileSerialPortAttachment::alloc(),
            &file_url(&spec.console_log),
            true,
        )
        .map_err(|e| ns_err("console log", e))?;
        let serial = VZVirtioConsoleDeviceSerialPortConfiguration::new();
        serial.setAttachment(Some(&console));
        config.setSerialPorts(&NSArray::from_retained_slice(&[Retained::into_super(serial)]));

        config.setSocketDevices(&NSArray::from_retained_slice(&[Retained::into_super(
            VZVirtioSocketDeviceConfiguration::new(),
        )]));

        let mut fs: Vec<Retained<VZDirectorySharingDeviceConfiguration>> = Vec::new();
        if !spec.shares.is_empty() {
            let names: Vec<Retained<NSString>> = spec.shares.iter().map(|s| NSString::from_str(&s.name)).collect();
            let dirs: Vec<Retained<VZSharedDirectory>> = spec
                .shares
                .iter()
                .map(|s| VZSharedDirectory::initWithURL_readOnly(VZSharedDirectory::alloc(), &file_url(&s.host_path), false))
                .collect();
            let keys: Vec<&NSString> = names.iter().map(|n| &**n).collect();
            let dict = NSDictionary::from_retained_objects(&keys, &dirs);
            let multi = VZMultipleDirectoryShare::initWithDirectories(VZMultipleDirectoryShare::alloc(), &dict);
            let dev = VZVirtioFileSystemDeviceConfiguration::initWithTag(
                VZVirtioFileSystemDeviceConfiguration::alloc(),
                &NSString::from_str(HOST_SHARE_TAG),
            );
            dev.setShare(Some(&multi));
            fs.push(Retained::into_super(dev));
        }
        if spec.rosetta {
            let share = VZLinuxRosettaDirectoryShare::initWithError(VZLinuxRosettaDirectoryShare::alloc())
                .map_err(|e| ns_err("rosetta share", e))?;
            let dev = VZVirtioFileSystemDeviceConfiguration::initWithTag(
                VZVirtioFileSystemDeviceConfiguration::alloc(),
                &NSString::from_str(ROSETTA_TAG),
            );
            dev.setShare(Some(&share));
            fs.push(Retained::into_super(dev));
        }
        config.setDirectorySharingDevices(&NSArray::from_retained_slice(&fs));

        config.validateWithError().map_err(|e| {
            ns_err(
                "invalid vm configuration (is the binary signed with the com.apple.security.virtualization entitlement? see vm/sign.sh)",
                e,
            )
        })?;
        Ok(config)
    }
}
