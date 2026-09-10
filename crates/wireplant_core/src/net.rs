//! Parsing `host:port` the one way every crate does it.

use std::fmt;

/// Why an endpoint string could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrError {
    /// No colon at all, so there is no port to find.
    MissingPort(String),
    /// A colon with something that is not a number after it.
    BadPort(String),
}

impl fmt::Display for AddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddrError::MissingPort(addr) => {
                write!(f, "'{addr}' has no port; write it as host:port")
            }
            AddrError::BadPort(addr) => {
                write!(f, "'{addr}' does not end in a valid port number")
            }
        }
    }
}

impl std::error::Error for AddrError {}

/// Splits `host:port` into its parts.
///
/// `default_port` covers the bare-host case only. A colon with a bad port after
/// it is an error rather than a fallback, because `127.0.0.1:1883x` silently
/// becoming port 1883 is how a typo reaches production. An empty host means
/// "every interface", which is what a bare `:5020` in a config file means.
pub fn parse_host_port(addr: &str, default_port: u16) -> Result<(String, u16), AddrError> {
    match addr.rsplit_once(':') {
        None if addr.trim().is_empty() => Ok(("0.0.0.0".to_string(), default_port)),
        None => Ok((addr.to_string(), default_port)),
        Some((host, port)) => {
            let port = port
                .trim()
                .parse::<u16>()
                .map_err(|_| AddrError::BadPort(addr.to_string()))?;
            let host = if host.trim().is_empty() {
                "0.0.0.0".to_string()
            } else {
                host.to_string()
            };
            Ok((host, port))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_and_port_split_on_the_last_colon() {
        assert_eq!(
            parse_host_port("127.0.0.1:1883", 9999).unwrap(),
            ("127.0.0.1".to_string(), 1883)
        );
        // IPv6 literals keep their colons in the host half.
        assert_eq!(
            parse_host_port("[::1]:1883", 9999).unwrap(),
            ("[::1]".to_string(), 1883)
        );
    }

    #[test]
    fn a_bare_host_takes_the_default_port_and_an_empty_host_is_every_interface() {
        assert_eq!(
            parse_host_port("mqtt.internal", 1883).unwrap(),
            ("mqtt.internal".to_string(), 1883)
        );
        assert_eq!(
            parse_host_port(":5020", 1883).unwrap(),
            ("0.0.0.0".to_string(), 5020)
        );
    }

    #[test]
    fn a_port_that_is_not_a_number_is_an_error_not_a_default() {
        let err = parse_host_port("127.0.0.1:1883x", 1883).unwrap_err();
        assert_eq!(err, AddrError::BadPort("127.0.0.1:1883x".to_string()));
        assert!(err.to_string().contains("valid port"));
    }
}
