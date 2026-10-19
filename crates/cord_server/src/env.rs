//! Server env validation.
//!
//! Read once at startup ([`CordEnv::from_process`] runs in
//! [`crate::runtime::run_app`] before the router is built), with no global
//! cached env. Source order: process env, then defaults. There is no `.env`
//! file resolution here; operators set vars through systemd, docker, direnv,
//! or the shell.
//!
//! The spine-wide names (`DATABASE_URL`, `HOST`, `PORT`,
//! `FUZ_ALLOWED_ORIGINS`, `FUZ_BOOTSTRAP_TOKEN_PATH`,
//! `SECRET_FUZ_COOKIE_KEYS`) are shared by every fuz spine server; the
//! cord-specific ones carry the `CORD_` prefix.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;

use crate::error::StartupError;
use crate::runtime::DEFAULT_ADDR;

/// Validated environment for `cordd`.
#[derive(Clone)]
pub struct CordEnv {
    /// `DATABASE_URL` for the Postgres pool. Absent means boot-only mode: the
    /// binary still serves `/health`, so a smoke test needs no database.
    pub database_url: Option<String>,

    /// Optional explicit bind address from `HOST` / `PORT`. Falls back to the
    /// caller's default address when both are unset.
    pub bind_addr: Option<SocketAddr>,

    /// Origin allowlist from `FUZ_ALLOWED_ORIGINS`, comma-separated. Required:
    /// [`fuz_http::AllowedOrigins::parse`] refuses a blank value and every
    /// malformed entry, so the boot fails naming each.
    pub allowed_origins: fuz_http::AllowedOrigins,

    /// Trusted reverse-proxy hops for `X-Forwarded-For` resolution, from
    /// `CORD_TRUSTED_PROXIES` (comma-separated IPs or CIDRs); empty when
    /// unset. A malformed entry fails the boot rather than silently dropping
    /// the list, which would record the proxy's IP (not the client's) in
    /// rate-limit keys and the audit log.
    pub trusted_proxies: Vec<fuz_http::ParsedProxy>,

    /// Bootstrap token file path from `FUZ_BOOTSTRAP_TOKEN_PATH`, with a
    /// leading `~/` resolved against `HOME`.
    pub bootstrap_token_path: Option<PathBuf>,

    /// Raw `SECRET_FUZ_COOKIE_KEYS` value (cookie-signing keys). Validated
    /// when the app is built, since boot-only mode needs none. Redacted from
    /// `Debug`.
    pub cookie_keys: Option<String>,
}

impl std::fmt::Debug for CordEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CordEnv")
            .field("has_database_url", &self.database_url.is_some())
            .field("bind_addr", &self.bind_addr)
            .field("allowed_origins", &self.allowed_origins)
            .field("trusted_proxies_count", &self.trusted_proxies.len())
            .field("bootstrap_token_path", &self.bootstrap_token_path)
            .field("has_cookie_keys", &self.cookie_keys.is_some())
            .finish()
    }
}

impl CordEnv {
    /// Read and validate the env of the current process.
    ///
    /// # Errors
    ///
    /// [`StartupError::Config`] on a missing required var or a malformed
    /// value.
    pub fn from_process() -> Result<Self, StartupError> {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    /// Read and validate from an arbitrary var lookup. [`Self::from_process`]
    /// injects `std::env::var`; tests inject a map, so env reading is verified
    /// without mutating the (racy) global process env.
    pub(crate) fn from_vars<F: Fn(&str) -> Option<String>>(get: F) -> Result<Self, StartupError> {
        let database_url = get("DATABASE_URL").filter(|s| !s.trim().is_empty());
        let bind_addr = resolve_bind_addr(&get)?;

        // Fail loud: a missing or malformed allowlist refuses the boot, naming
        // every refused entry, rather than leaving the origin gate on the
        // account routes, `/api/rpc`, and the WebSocket upgrade unusable.
        let allowed_origins =
            fuz_http::AllowedOrigins::parse(&get("FUZ_ALLOWED_ORIGINS").unwrap_or_default())
                .map_err(|e| StartupError::Config(format!("FUZ_ALLOWED_ORIGINS: {e}")))?;

        let trusted_proxies = get("CORD_TRUSTED_PROXIES")
            .filter(|s| !s.trim().is_empty())
            .map(|s| fuz_http::parse_proxy_list(&s))
            .transpose()
            .map_err(|e| StartupError::Config(format!("CORD_TRUSTED_PROXIES: {e}")))?
            .unwrap_or_default();

        let home = get("HOME");
        let bootstrap_token_path = get("FUZ_BOOTSTRAP_TOKEN_PATH")
            .filter(|s| !s.trim().is_empty())
            .map(|s| resolve_tilde(&s, home.as_deref()));

        let cookie_keys = get("SECRET_FUZ_COOKIE_KEYS");

        Ok(Self {
            database_url,
            bind_addr,
            allowed_origins,
            trusted_proxies,
            bootstrap_token_path,
            cookie_keys,
        })
    }
}

/// Resolve the optional `HOST` + numeric `PORT` pair into a bind address.
///
/// Returns `None` when neither is set, so the caller's default address
/// applies. A set `HOST` resolves via `ToSocketAddrs` (`localhost`,
/// `127.0.0.1`, ...), first address wins; a missing half is filled from
/// [`DEFAULT_ADDR`].
fn resolve_bind_addr<F: Fn(&str) -> Option<String>>(
    get: &F,
) -> Result<Option<SocketAddr>, StartupError> {
    let host = get("HOST").filter(|s| !s.trim().is_empty());
    let port = match get("PORT").filter(|s| !s.trim().is_empty()) {
        None => None,
        Some(s) => Some(
            s.trim()
                .parse::<u16>()
                .map_err(|e| StartupError::Config(format!("PORT: {e}")))?,
        ),
    };
    if host.is_none() && port.is_none() {
        return Ok(None);
    }
    let host = host.unwrap_or_else(|| DEFAULT_ADDR.ip().to_string());
    let port = port.unwrap_or_else(|| DEFAULT_ADDR.port());
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| StartupError::Config(format!("HOST/PORT '{host}:{port}': {e}")))?
        .next()
        .ok_or_else(|| StartupError::Config(format!("HOST '{host}' resolved to no addresses")))?;
    Ok(Some(addr))
}

/// Resolve a leading `~/` against `home`. Falls through unchanged when `home`
/// is unset.
fn resolve_tilde(path: &str, home: Option<&str>) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build an env from explicit pairs via the injectable `from_vars`, with a
    /// default origin allowlist so each test states only what it's about.
    fn env_from(pairs: &[(&str, &str)]) -> CordEnv {
        let mut map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        map.entry("FUZ_ALLOWED_ORIGINS".to_owned())
            .or_insert_with(|| "http://localhost:*".to_owned());
        CordEnv::from_vars(|k| map.get(k).cloned()).expect("env should parse")
    }

    #[test]
    fn reads_spine_var_names() {
        let env = env_from(&[
            ("DATABASE_URL", "postgres://localhost/cord"),
            ("HOST", "127.0.0.1"),
            ("PORT", "14061"),
            ("FUZ_ALLOWED_ORIGINS", "http://a.example, http://b.example"),
            ("FUZ_BOOTSTRAP_TOKEN_PATH", "/tmp/bt"),
            ("SECRET_FUZ_COOKIE_KEYS", "k"),
        ]);
        assert_eq!(
            env.database_url.as_deref(),
            Some("postgres://localhost/cord")
        );
        assert_eq!(env.bind_addr, Some("127.0.0.1:14061".parse().unwrap()));
        let origins: Vec<&str> = env
            .allowed_origins
            .patterns()
            .iter()
            .map(fuz_http::OriginPattern::as_str)
            .collect();
        assert_eq!(origins, ["http://a.example", "http://b.example"]);
        assert_eq!(env.bootstrap_token_path, Some(PathBuf::from("/tmp/bt")));
        assert_eq!(env.cookie_keys.as_deref(), Some("k"));
    }

    #[test]
    fn empty_database_url_is_boot_only() {
        assert_eq!(env_from(&[("DATABASE_URL", "  ")]).database_url, None);
        assert_eq!(env_from(&[]).database_url, None);
    }

    #[test]
    fn bind_addr_none_when_host_and_port_unset() {
        assert_eq!(env_from(&[]).bind_addr, None);
    }

    #[test]
    fn partial_bind_fills_missing_half_from_default_addr() {
        let env = env_from(&[("PORT", "12345")]);
        assert_eq!(
            env.bind_addr,
            Some(SocketAddr::new(DEFAULT_ADDR.ip(), 12345))
        );
    }

    #[test]
    fn malformed_port_hard_fails() {
        let err = CordEnv::from_vars(|k| match k {
            "FUZ_ALLOWED_ORIGINS" => Some("http://localhost".to_owned()),
            "PORT" => Some("not-a-port".to_owned()),
            _ => None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("PORT"), "got: {err}");
    }

    #[test]
    fn malformed_trusted_proxies_hard_fails() {
        let err = CordEnv::from_vars(|k| match k {
            "FUZ_ALLOWED_ORIGINS" => Some("http://localhost".to_owned()),
            "CORD_TRUSTED_PROXIES" => Some("not-an-ip".to_owned()),
            _ => None,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("CORD_TRUSTED_PROXIES"),
            "malformed proxy list must hard-fail naming the var, got: {err}"
        );
    }

    #[test]
    fn trusted_proxies_unset_is_empty_and_valid_entry_parses() {
        assert!(env_from(&[]).trusted_proxies.is_empty());
        assert_eq!(
            env_from(&[("CORD_TRUSTED_PROXIES", "127.0.0.1")])
                .trusted_proxies
                .len(),
            1
        );
    }

    #[test]
    fn missing_or_empty_allowed_origins_hard_fails() {
        let absent = CordEnv::from_vars(|_| None).unwrap_err();
        assert!(
            absent.to_string().contains("FUZ_ALLOWED_ORIGINS"),
            "absent allowlist must hard-fail naming FUZ_ALLOWED_ORIGINS, got: {absent}"
        );
        let empty = CordEnv::from_vars(|k| (k == "FUZ_ALLOWED_ORIGINS").then(|| "  , ".to_owned()))
            .unwrap_err();
        assert!(
            empty.to_string().contains("FUZ_ALLOWED_ORIGINS"),
            "whitespace-only allowlist must hard-fail, got: {empty}"
        );
    }

    #[test]
    fn a_malformed_allowed_origin_hard_fails_naming_the_entry() {
        let err = CordEnv::from_vars(|k| {
            (k == "FUZ_ALLOWED_ORIGINS")
                .then(|| "https://cord.example,https://cord.example/".to_owned())
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("FUZ_ALLOWED_ORIGINS: "), "{err}");
        assert!(err.contains("entry 2"), "{err}");
    }

    #[test]
    fn bootstrap_token_path_resolves_tilde_against_home() {
        let env = env_from(&[
            ("HOME", "/home/cord"),
            ("FUZ_BOOTSTRAP_TOKEN_PATH", "~/.cord/secret_bootstrap_token"),
        ]);
        assert_eq!(
            env.bootstrap_token_path,
            Some(PathBuf::from("/home/cord/.cord/secret_bootstrap_token"))
        );
        // without HOME the path is left as written
        let env = env_from(&[("FUZ_BOOTSTRAP_TOKEN_PATH", "~/t")]);
        assert_eq!(env.bootstrap_token_path, Some(PathBuf::from("~/t")));
    }

    #[test]
    fn debug_redacts_cookie_keys() {
        let env = env_from(&[("SECRET_FUZ_COOKIE_KEYS", "super-secret-value")]);
        let debug = format!("{env:?}");
        assert!(!debug.contains("super-secret-value"), "got: {debug}");
        assert!(debug.contains("has_cookie_keys: true"), "got: {debug}");
    }
}
