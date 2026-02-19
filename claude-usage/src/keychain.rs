use crate::error::{AppError, Result};
use security_framework::passwords::get_generic_password;

pub fn get_brave_password() -> Result<String> {
    let password_bytes =
        get_generic_password("Brave Safe Storage", "Brave").map_err(|e| AppError::Keychain {
            msg: e.to_string(),
        })?;

    String::from_utf8(password_bytes.to_vec()).map_err(|e| AppError::Keychain {
        msg: format!("password is not valid UTF-8: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // requires Brave installed + Keychain access
    fn keychain_returns_nonempty() {
        let pw = get_brave_password().expect("should read keychain");
        assert!(!pw.is_empty(), "password should not be empty");
    }
}
