use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{name} is not set; pass --{flag} or export {name} ({hint})")]
    MissingCredential {
        name: String,
        flag: String,
        hint: String,
    },

    #[error(
        "No Steam account given; pass --steam-id (or export STEAM_ID) with your 17-digit \
         SteamID64, or --steam-vanity (or export STEAM_VANITY) with the name from your profile \
         URL. Find your SteamID64 at https://steamcommunity.com/my/ (the number in the URL if \
         your profile has no custom URL), or in Steam under Account details."
    )]
    MissingSteamId,

    #[error(
        "Humble Bundle rejected the session cookie (HTTP {status}); copy a fresh \
         _simpleauth_sess value from the browser and pass it via --humble-session or \
         HUMBLE_SESSION"
    )]
    HumbleAuth { status: u16 },

    #[error("Steam rejected the API key (HTTP {status})")]
    SteamAuth { status: u16 },

    #[error(
        "Steam returned no games for {steamid}: the profile may be private; generate the key \
         while logged in as that account, or set Game details to Public in Steam privacy \
         settings"
    )]
    SteamEmpty { steamid: u64 },

    #[error("Steam could not resolve vanity name \"{name}\": {msg}")]
    VanityNotFound { name: String, msg: String },

    #[error("http error: {msg}")]
    Http { msg: String },

    #[error("json parse error: {msg}")]
    JsonParse { msg: String },

    #[error("dump error at {path}: {msg}")]
    Dump { path: String, msg: String },

    #[error("config error at {path}: {msg}")]
    Config { path: String, msg: String },

    #[error("{msg}")]
    Wizard { msg: String },
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Http { msg: e.to_string() }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::JsonParse { msg: e.to_string() }
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_missing_credential() {
        let e = AppError::MissingCredential {
            name: "HUMBLE_SESSION".into(),
            flag: "humble-session".into(),
            hint: "the _simpleauth_sess cookie from humblebundle.com".into(),
        };
        let s = e.to_string();
        assert!(s.contains("HUMBLE_SESSION is not set"));
        assert!(s.contains("--humble-session"));
        assert!(s.contains("_simpleauth_sess cookie"));
    }

    #[test]
    fn display_missing_steam_id() {
        let s = AppError::MissingSteamId.to_string();
        assert!(s.contains("--steam-id"));
        assert!(s.contains("--steam-vanity"));
        assert!(s.contains("steamcommunity.com/my"));
    }

    #[test]
    fn display_humble_auth() {
        let e = AppError::HumbleAuth { status: 302 };
        let s = e.to_string();
        assert!(s.contains("HTTP 302"));
        assert!(s.contains("_simpleauth_sess"));
    }

    #[test]
    fn display_steam_auth() {
        let e = AppError::SteamAuth { status: 401 };
        assert!(e.to_string().contains("HTTP 401"));
    }

    #[test]
    fn display_steam_empty() {
        let e = AppError::SteamEmpty {
            steamid: 7656119_000000000,
        };
        let s = e.to_string();
        assert!(s.contains("7656119000000000"));
        assert!(s.contains("Public"));
    }

    #[test]
    fn display_vanity_not_found() {
        let e = AppError::VanityNotFound {
            name: "nosuchname".into(),
            msg: "No match".into(),
        };
        let s = e.to_string();
        assert!(s.contains("\"nosuchname\""));
        assert!(s.contains("No match"));
    }

    #[test]
    fn display_http_error() {
        let e = AppError::Http {
            msg: "connection refused".into(),
        };
        assert!(e.to_string().contains("http error"));
        assert!(e.to_string().contains("connection refused"));
    }

    #[test]
    fn display_json_parse_error() {
        let e = AppError::JsonParse {
            msg: "unexpected token".into(),
        };
        assert!(e.to_string().contains("json parse error"));
        assert!(e.to_string().contains("unexpected token"));
    }

    #[test]
    fn display_dump_error() {
        let e = AppError::Dump {
            path: "dump/humble_orders.json".into(),
            msg: "permission denied".into(),
        };
        let s = e.to_string();
        assert!(s.contains("dump/humble_orders.json"));
        assert!(s.contains("permission denied"));
    }

    #[test]
    fn display_config_error() {
        let e = AppError::Config {
            path: "/tmp/config.json".into(),
            msg: "permission denied".into(),
        };
        let s = e.to_string();
        assert!(s.contains("/tmp/config.json"));
        assert!(s.contains("permission denied"));
    }

    #[test]
    fn display_wizard_error() {
        let e = AppError::Wizard {
            msg: "setup cancelled; pass the missing flags".into(),
        };
        assert!(e.to_string().contains("setup cancelled"));
    }

    #[test]
    fn from_serde_json_error() {
        let err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::JsonParse { .. }));
    }

    #[test]
    fn from_reqwest_error() {
        let err = reqwest::blocking::get("http://[::invalid]").unwrap_err();
        let app_err: AppError = err.into();
        assert!(matches!(app_err, AppError::Http { .. }));
    }
}
