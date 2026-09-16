//! Interactive setup for missing Humble/Steam credentials. Prompts go to the
//! provided writer (stderr in the CLI) so the report can still use stdout.

use crate::config::StoredConfig;
use crate::error::{AppError, Result};
use std::io::{BufRead, IsTerminal, Write};

/// Which required values still need to be collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub humble_session: bool,
    pub steam_key: bool,
    pub steam_account: bool,
}

impl Missing {
    pub fn from_config(cfg: &StoredConfig) -> Self {
        Self {
            humble_session: cfg.missing_humble_session(),
            steam_key: cfg.missing_steam_key(),
            steam_account: cfg.missing_steam_account(),
        }
    }
}

/// True when the wizard should run: a TTY on stdin, or
/// `HUMBLE_GAME_CHECK_WIZARD=1` (used by tests to pipe answers).
pub fn should_run() -> bool {
    match std::env::var("HUMBLE_GAME_CHECK_WIZARD").ok().as_deref() {
        Some("1") | Some("always") | Some("true") => true,
        Some("0") | Some("never") | Some("false") => false,
        _ => std::io::stdin().is_terminal(),
    }
}

/// Walks through each missing value, then asks whether to store what was
/// entered. Returns the collected values and whether the user said to save.
pub fn run<R: BufRead, W: Write>(
    input: &mut R,
    out: &mut W,
    missing: Missing,
) -> Result<(StoredConfig, bool)> {
    writeln!(
        out,
        "Missing credentials. This wizard will walk you through getting them.\n"
    )
    .map_err(wizard_io)?;

    let mut entered = StoredConfig::default();

    if missing.humble_session {
        entered.humble_session = Some(prompt_humble_session(input, out)?);
    }
    if missing.steam_key {
        entered.steam_key = Some(prompt_steam_key(input, out)?);
    }
    if missing.steam_account {
        prompt_steam_account(input, out, &mut entered)?;
    }

    let save = prompt_yes_no(
        input,
        out,
        "Store these for next time so you don't have to pass flags or env vars again? [y/N] ",
    )?;

    Ok((entered, save))
}

/// Prompts for the Humble `_simpleauth_sess` cookie and how to copy it.
fn prompt_humble_session<R: BufRead, W: Write>(input: &mut R, out: &mut W) -> Result<String> {
    writeln!(
        out,
        "Humble session cookie (--humble-session / HUMBLE_SESSION)\n\
         \n\
         Log into https://www.humblebundle.com in your browser, then copy the\n\
         _simpleauth_sess cookie:\n\
           Chrome:  DevTools > Application > Cookies > humblebundle.com\n\
           Firefox: Storage tab > Cookies > humblebundle.com\n\
           Safari:  Storage tab > Cookies > humblebundle.com\n\
         This is a full login session. Keep it private; it will expire.\n"
    )
    .map_err(wizard_io)?;
    read_value(input, out, "HUMBLE_SESSION: ")
}

/// Prompts for a Steam Web API key and where to generate one.
fn prompt_steam_key<R: BufRead, W: Write>(input: &mut R, out: &mut W) -> Result<String> {
    writeln!(
        out,
        "Steam Web API key (--steam-key / STEAM_API_KEY)\n\
         \n\
         Log into Steam as the account you want to check, then generate a key at:\n\
           https://steamcommunity.com/dev/apikey\n\
         Any domain name string works. A friend's private library is only readable\n\
         when the key belongs to that same account.\n"
    )
    .map_err(wizard_io)?;
    read_value(input, out, "STEAM_API_KEY: ")
}

/// Asks SteamID64 vs vanity name, then prompts for that value.
fn prompt_steam_account<R: BufRead, W: Write>(
    input: &mut R,
    out: &mut W,
    entered: &mut StoredConfig,
) -> Result<()> {
    writeln!(
        out,
        "Steam account (--steam-id / STEAM_ID, or --steam-vanity / STEAM_VANITY)\n\
         \n\
         You need one of:\n\
           1) SteamID64, the 17-digit number. Find it at https://steamcommunity.com/my/\n\
              (the number in the URL if your profile has no custom URL), or under\n\
              Steam > Account details.\n\
           2) Vanity name, from https://steamcommunity.com/id/<name>. This is resolved\n\
              through the Steam API, so it only works when looking up your own account.\n"
    )
    .map_err(wizard_io)?;

    loop {
        let choice = read_value(input, out, "Choose 1 or 2: ")?;
        match choice.as_str() {
            "1" => {
                entered.steam_id = Some(read_steam_id(input, out)?);
                return Ok(());
            }
            "2" => {
                entered.steam_vanity = Some(read_value(input, out, "STEAM_VANITY: ")?);
                return Ok(());
            }
            _ => writeln!(out, "Please enter 1 or 2.").map_err(wizard_io)?,
        }
    }
}

/// Reads a SteamID64, re-prompting until the input is a number.
fn read_steam_id<R: BufRead, W: Write>(input: &mut R, out: &mut W) -> Result<u64> {
    loop {
        let raw = read_value(input, out, "STEAM_ID: ")?;
        match raw.parse::<u64>() {
            Ok(id) => return Ok(id),
            Err(_) => writeln!(out, "That isn't a number. Paste the 17-digit SteamID64.")
                .map_err(wizard_io)?,
        }
    }
}

/// Reads a non-empty line, stripping surrounding quotes from a copy-paste.
fn read_value<R: BufRead, W: Write>(input: &mut R, out: &mut W, prompt: &str) -> Result<String> {
    loop {
        write!(out, "{prompt}").map_err(wizard_io)?;
        out.flush().map_err(wizard_io)?;
        let mut line = String::new();
        let n = input.read_line(&mut line).map_err(wizard_io)?;
        if n == 0 {
            return Err(AppError::Wizard {
                msg: "setup cancelled; pass the missing flags or export the matching env vars"
                    .into(),
            });
        }
        let value = strip_quotes(line.trim());
        if !value.is_empty() {
            return Ok(value);
        }
        writeln!(out, "Please enter a value.").map_err(wizard_io)?;
    }
}

/// y/yes (any case) is true; anything else, including empty, is false.
fn prompt_yes_no<R: BufRead, W: Write>(input: &mut R, out: &mut W, prompt: &str) -> Result<bool> {
    write!(out, "{prompt}").map_err(wizard_io)?;
    out.flush().map_err(wizard_io)?;
    let mut line = String::new();
    let n = input.read_line(&mut line).map_err(wizard_io)?;
    if n == 0 {
        return Ok(false);
    }
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

/// Removes a matching pair of wrapping single or double quotes.
fn strip_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

fn wizard_io(e: std::io::Error) -> AppError {
    AppError::Wizard {
        msg: format!("could not read input: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run_with(input: &str, missing: Missing) -> Result<(StoredConfig, bool, String)> {
        let mut cursor = Cursor::new(input.to_string());
        let mut out = Vec::new();
        let (cfg, save) = run(&mut cursor, &mut out, missing)?;
        Ok((cfg, save, String::from_utf8(out).unwrap()))
    }

    fn all_missing() -> Missing {
        Missing {
            humble_session: true,
            steam_key: true,
            steam_account: true,
        }
    }

    #[test]
    fn missing_from_empty_config() {
        let m = Missing::from_config(&StoredConfig::default());
        assert!(m.humble_session && m.steam_key && m.steam_account);
    }

    #[test]
    fn missing_from_complete_config() {
        let m = Missing::from_config(&StoredConfig {
            humble_session: Some("s".into()),
            steam_key: Some("k".into()),
            steam_id: Some(1),
            steam_vanity: None,
        });
        assert!(!m.humble_session && !m.steam_key && !m.steam_account);
    }

    #[test]
    fn collects_session_key_and_steam_id_then_saves() {
        let (cfg, save, out) = run_with(
            "sess-cookie\nsteam-key\n1\n76561197960287930\ny\n",
            all_missing(),
        )
        .unwrap();
        assert!(save);
        assert_eq!(cfg.humble_session.as_deref(), Some("sess-cookie"));
        assert_eq!(cfg.steam_key.as_deref(), Some("steam-key"));
        assert_eq!(cfg.steam_id, Some(76561197960287930));
        assert!(cfg.steam_vanity.is_none());
        assert!(out.contains("_simpleauth_sess"));
        assert!(out.contains("steamcommunity.com/dev/apikey"));
        assert!(out.contains("steamcommunity.com/my/"));
        assert!(out.contains("Store these for next time"));
    }

    #[test]
    fn collects_vanity_and_declines_save() {
        let (cfg, save, _) = run_with("sess\nkey\n2\nmyvanity\nn\n", all_missing()).unwrap();
        assert!(!save);
        assert_eq!(cfg.steam_vanity.as_deref(), Some("myvanity"));
        assert!(cfg.steam_id.is_none());
    }

    #[test]
    fn empty_default_on_save_prompt_is_no() {
        let (_, save, _) = run_with("sess\nkey\n2\nvanity\n\n", all_missing()).unwrap();
        assert!(!save);
    }

    #[test]
    fn yes_is_case_insensitive() {
        let (_, save, _) = run_with("sess\nkey\n2\nvanity\nYES\n", all_missing()).unwrap();
        assert!(save);
    }

    #[test]
    fn strips_wrapping_quotes() {
        let (cfg, _, _) = run_with(
            "\"quoted-sess\"\n'quoted-key'\n2\nvanity\nn\n",
            all_missing(),
        )
        .unwrap();
        assert_eq!(cfg.humble_session.as_deref(), Some("quoted-sess"));
        assert_eq!(cfg.steam_key.as_deref(), Some("quoted-key"));
    }

    #[test]
    fn reprompts_on_empty_then_invalid_choice_and_bad_steam_id() {
        let (cfg, save, out) = run_with(
            "\nsess\nkey\n9\n1\nabc\n7656119000000000\ny\n",
            all_missing(),
        )
        .unwrap();
        assert!(save);
        assert_eq!(cfg.humble_session.as_deref(), Some("sess"));
        assert_eq!(cfg.steam_id, Some(7656119000000000));
        assert!(out.contains("Please enter a value."));
        assert!(out.contains("Please enter 1 or 2."));
        assert!(out.contains("17-digit SteamID64"));
    }

    #[test]
    fn only_prompts_for_what_is_missing() {
        let missing = Missing {
            humble_session: false,
            steam_key: true,
            steam_account: false,
        };
        let (cfg, save, out) = run_with("only-key\ny\n", missing).unwrap();
        assert!(save);
        assert!(cfg.humble_session.is_none());
        assert_eq!(cfg.steam_key.as_deref(), Some("only-key"));
        assert!(cfg.steam_id.is_none());
        assert!(!out.contains("_simpleauth_sess"));
        assert!(!out.contains("Choose 1 or 2"));
    }

    #[test]
    fn eof_before_answers_is_cancelled() {
        let err = run_with("", all_missing()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("setup cancelled"));
        assert!(msg.contains("flags") || msg.contains("env vars"));
    }

    #[test]
    fn eof_on_save_prompt_means_do_not_save() {
        let (cfg, save, _) = run_with("sess\nkey\n2\nvanity", all_missing()).unwrap();
        assert!(!save);
        assert_eq!(cfg.steam_vanity.as_deref(), Some("vanity"));
    }

    #[test]
    fn strip_quotes_leaves_unquoted_and_unbalanced() {
        assert_eq!(strip_quotes("plain"), "plain");
        assert_eq!(strip_quotes("\"mid"), "\"mid");
        assert_eq!(strip_quotes("'a'"), "a");
    }
}
