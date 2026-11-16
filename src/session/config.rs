use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub version: u32,
    pub name: String,
    pub artifact_id: String,
    pub worker_argv: Vec<String>,
    pub cold_argv: Vec<String>,
    #[serde(default)]
    pub cold_if_env_nonempty: Vec<String>,
    #[serde(default)]
    pub warm_ignore_env: Vec<String>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default = "startup_timeout")]
    pub startup_timeout_ms: u64,
    #[serde(default = "session_timeout")]
    pub session_timeout_ms: u64,
    #[serde(default = "cleanup_timeout")]
    pub cleanup_timeout_ms: u64,
    pub context: Option<ContextOverrides>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextOverrides {
    pub cwd: Option<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, Option<String>>,
    pub umask: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Resource {
    TcpListener { host: String, port: u16 },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub protocol: u32,
    pub session_id: String,
    pub artifact_id: String,
    pub token: String,
    pub connect: Connect,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub host: String,
    pub port: u16,
}

fn startup_timeout() -> u64 {
    30_000
}

fn session_timeout() -> u64 {
    300_000
}

fn cleanup_timeout() -> u64 {
    6_000
}

/// How long an owner waits for custody release after stopping: twice the per-group cleanup budget plus a fixed margin.
///
/// The session host waits exactly this long, and batch sizes each wave's lifetime from the same value, so the two must not diverge.
pub(super) fn cleanup_grace(cleanup_timeout_ms: u64) -> Option<Duration> {
    Duration::from_millis(cleanup_timeout_ms)
        .checked_mul(2)
        .and_then(|value| value.checked_add(Duration::from_millis(500)))
}

impl Binding {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path)
            .map_err(|error| format!("cannot read binding configuration ({:?})", error.kind()))?;
        let binding: Self = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "invalid binding JSON at line {}, column {}",
                error.line(),
                error.column()
            )
        })?;
        binding.validate()?;
        Ok(binding)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported binding version".to_string());
        }
        if self.name.is_empty()
            || matches!(self.name.as_str(), "mdrun" | "." | "..")
            || self.name.contains(['/', '\\', '\0'])
        {
            return Err(
                "binding name must be a single executable name other than mdrun".to_string(),
            );
        }
        if self.artifact_id.is_empty() {
            return Err("binding artifact_id must be nonempty".to_string());
        }
        validate_argv("worker_argv", &self.worker_argv)?;
        validate_argv("cold_argv", &self.cold_argv)?;
        for duration in [
            self.startup_timeout_ms,
            self.session_timeout_ms,
            self.cleanup_timeout_ms,
        ] {
            if duration == 0 || duration > i64::MAX as u64 {
                return Err(
                    "binding timeouts must be positive representable milliseconds".to_string(),
                );
            }
        }
        for name in &self.cold_if_env_nonempty {
            validate_environment_name(name)?;
        }
        let mut ignored = HashSet::new();
        for name in &self.warm_ignore_env {
            if !matches!(name.as_str(), "SHLVL" | "_") || !ignored.insert(name) {
                return Err("warm_ignore_env supports only distinct SHLVL and _ names".to_string());
            }
        }
        let mut endpoints = HashSet::new();
        for resource in &self.resources {
            if !endpoints.insert(resource.socket_addr()?) {
                return Err("duplicate binding resource endpoint".to_string());
            }
        }
        if let Some(context) = &self.context {
            if context
                .cwd
                .as_ref()
                .is_some_and(|cwd| !Path::new(cwd).is_absolute() || cwd.contains('\0'))
            {
                return Err("context cwd must be an absolute native path".to_string());
            }
            if context.umask.is_some_and(|umask| umask > 511) {
                return Err("context umask must be between 0 and 511".to_string());
            }
            for (name, value) in &context.environment {
                validate_environment_name(name)?;
                if value.as_ref().is_some_and(|value| value.contains('\0')) {
                    return Err("context environment values cannot contain NUL".to_string());
                }
            }
        }
        Ok(())
    }
}

fn validate_argv(field: &str, argv: &[String]) -> Result<(), String> {
    if argv.is_empty() || !Path::new(&argv[0]).is_absolute() {
        return Err(format!(
            "{field} must start with an absolute executable path"
        ));
    }
    if argv.iter().any(|argument| argument.contains('\0')) {
        return Err(format!("{field} cannot contain NUL"));
    }
    Ok(())
}

fn validate_environment_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(
            "environment variable names must be nonempty and contain neither '=' nor NUL"
                .to_string(),
        );
    }
    Ok(())
}

impl Resource {
    pub fn socket_addr(&self) -> Result<SocketAddr, String> {
        match self {
            Self::TcpListener { host, port } => {
                let address: IpAddr = host.parse().map_err(|_| {
                    "resource host must be a canonical loopback IP address".to_string()
                })?;
                if !address.is_loopback() || address.to_string() != *host || *port == 0 {
                    return Err(
                        "resource endpoint must be a canonical loopback IP and nonzero port"
                            .to_string(),
                    );
                }
                Ok(SocketAddr::new(address, *port))
            }
        }
    }
}

impl fmt::Debug for WorkerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerConfig")
            .field("protocol", &self.protocol)
            .field("session_id", &self.session_id)
            .field("artifact_id", &self.artifact_id)
            .field("token", &"[redacted]")
            .field("connect", &self.connect)
            .finish()
    }
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.protocol != 1 {
            return Err("unsupported worker protocol".to_string());
        }
        if self.session_id.is_empty() || self.artifact_id.is_empty() || self.token.is_empty() {
            return Err("worker session_id, artifact_id and token must be nonempty".to_string());
        }
        if self.connect.host != "127.0.0.1" || self.connect.port == 0 {
            return Err(
                "worker connect endpoint must use 127.0.0.1 and a nonzero port".to_string(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::PathBuf;

    struct TemporaryConfig(PathBuf);

    impl TemporaryConfig {
        fn new(value: &Value) -> Self {
            let path =
                std::env::temp_dir().join(format!("mdrun-config-{}.json", uuid::Uuid::new_v4()));
            fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            Self(path)
        }
    }

    impl Drop for TemporaryConfig {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn binding_json() -> Value {
        json!({
            "version": 1,
            "name": "example-tool",
            "artifact_id": "sha256:prepared",
            "worker_argv": ["/prepared/worker", "--fixed"],
            "cold_argv": ["/prepared/launcher"]
        })
    }

    fn validate(json: Value) -> Result<Binding, String> {
        let binding: Binding = serde_json::from_value(json).map_err(|error| error.to_string())?;
        binding.validate()?;
        Ok(binding)
    }

    #[test]
    fn manifest_defaults_require_no_executable_discovery() {
        let binding = validate(binding_json()).unwrap();
        assert_eq!(binding.startup_timeout_ms, 30_000);
        assert_eq!(binding.session_timeout_ms, 300_000);
        assert_eq!(binding.cleanup_timeout_ms, 6_000);
        assert!(binding.resources.is_empty());
        assert!(binding.cold_if_env_nonempty.is_empty());
        assert_eq!(
            serde_json::to_value(&binding).unwrap()["warm_ignore_env"],
            json!([])
        );
        assert!(binding.context.is_none());
    }

    #[test]
    fn warm_environment_projection_accepts_only_distinct_explicit_shell_keys() {
        for names in [
            json!([]),
            json!(["SHLVL"]),
            json!(["_"]),
            json!(["SHLVL", "_"]),
            json!(["_", "SHLVL"]),
        ] {
            let mut json = binding_json();
            json["warm_ignore_env"] = names.clone();
            let binding = validate(json).unwrap();
            assert_eq!(
                serde_json::to_value(binding).unwrap()["warm_ignore_env"],
                names
            );
        }
        for names in [
            json!(["SHLVL", "SHLVL"]),
            json!(["_", "_"]),
            json!(["SHLVL", "_", "SHLVL"]),
            json!(["PATH"]),
            json!(["PWD"]),
            json!(["VALUE"]),
            json!(["MDRUN_SESSION"]),
            json!(["shlvl"]),
            json!(["SHLVL "]),
            json!([""]),
            json!([1]),
            json!("SHLVL"),
            Value::Null,
        ] {
            let mut json = binding_json();
            json["warm_ignore_env"] = names;
            assert!(validate(json).is_err(), "accepted warm projection names");
        }
    }

    #[test]
    fn file_loading_validates_before_exposing_configuration() {
        let file = TemporaryConfig::new(&binding_json());
        assert!(Binding::from_file(&file.0).is_ok());
        let mut invalid = binding_json();
        invalid["version"] = json!(2);
        fs::write(&file.0, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(Binding::from_file(&file.0).is_err());
    }

    #[test]
    fn bindings_reject_invalid_names_and_executable_vectors() {
        for name in ["", "mdrun", ".", "..", "a/b", "a\\b", "a\0b"] {
            let mut json = binding_json();
            json["name"] = json!(name);
            assert!(validate(json).is_err(), "accepted name {name:?}");
        }
        for field in ["worker_argv", "cold_argv"] {
            for argv in [
                json!([]),
                json!(["tool"]),
                json!([""]),
                json!(["/tool", "a\0b"]),
            ] {
                let mut json = binding_json();
                json[field] = argv;
                assert!(validate(json).is_err(), "accepted {field}");
            }
        }
        let mut json = binding_json();
        json["artifact_id"] = json!("");
        assert!(validate(json).is_err());
    }

    #[test]
    fn binding_schema_and_version_are_strict() {
        for (field, value) in [
            ("version", json!(2)),
            ("unknown", json!(1)),
            ("name", json!(1)),
            ("cold_argv", json!([1])),
            ("startup_timeout_ms", json!(-1)),
            ("cleanup_timeout_ms", json!(1.5)),
        ] {
            let mut json = binding_json();
            json[field] = value;
            assert!(validate(json).is_err(), "accepted {field}");
        }
    }

    #[test]
    fn durations_are_positive_and_deadline_representable() {
        for field in [
            "startup_timeout_ms",
            "session_timeout_ms",
            "cleanup_timeout_ms",
        ] {
            for duration in [0, u64::MAX] {
                let mut json = binding_json();
                json[field] = json!(duration);
                assert!(validate(json).is_err(), "accepted {field}={duration}");
            }
        }
    }

    #[test]
    fn cleanup_grace_covers_the_complete_binding_timeout_domain() {
        let largest = i64::MAX as u64;
        for timeout in (largest - 250)..=largest {
            let mut json = binding_json();
            json["cleanup_timeout_ms"] = json!(timeout);
            let binding = validate(json).unwrap();
            assert_eq!(
                cleanup_grace(binding.cleanup_timeout_ms)
                    .unwrap_or_else(|| panic!("rejected valid cleanup timeout {timeout}"))
                    .as_millis(),
                u128::from(timeout) * 2 + 500
            );
        }
    }

    #[test]
    fn resources_are_canonical_exclusive_loopback_endpoints() {
        let mut json = binding_json();
        json["resources"] = json!([
            {"kind":"tcp_listener","host":"127.0.0.1","port":49152},
            {"kind":"tcp_listener","host":"::1","port":49152}
        ]);
        assert!(validate(json.clone()).is_ok());
        for resource in [
            json!({"kind":"tcp_listener","host":"localhost","port":49152}),
            json!({"kind":"tcp_listener","host":"0.0.0.0","port":49152}),
            json!({"kind":"tcp_listener","host":"192.0.2.1","port":49152}),
            json!({"kind":"tcp_listener","host":"::ffff:127.0.0.1","port":49152}),
            json!({"kind":"tcp_listener","host":"0:0:0:0:0:0:0:1","port":49152}),
            json!({"kind":"tcp_listener","host":"127.0.0.1","port":0}),
            json!({"kind":"tcp_listener","host":"127.0.0.1","port":65536}),
            json!({"kind":"file_lock","host":"127.0.0.1","port":49152}),
        ] {
            json["resources"] = json!([resource]);
            assert!(validate(json.clone()).is_err(), "accepted resource {json}");
        }
        json["resources"] = json!([
            {"kind":"tcp_listener","host":"127.0.0.1","port":49152},
            {"kind":"tcp_listener","host":"127.0.0.1","port":49152}
        ]);
        assert!(validate(json).is_err());
    }

    #[test]
    fn context_is_a_patch_and_validates_native_names_and_umask() {
        let mut json = binding_json();
        json["context"] = json!({
            "cwd":"/prepared/run",
            "environment":{"SET":"value", "REMOVE":null},
            "umask":511
        });
        assert!(validate(json.clone()).is_ok());
        for context in [
            json!({"cwd":"relative"}),
            json!({"cwd":"/a\0b"}),
            json!({"umask":512}),
            json!({"environment":{"BAD=KEY":"value"}}),
            json!({"environment":{"":"value"}}),
            json!({"environment":{"KEY":"a\0b"}}),
            json!({"unknown":true}),
        ] {
            json["context"] = context;
            assert!(validate(json.clone()).is_err());
        }
        for names in [json!([""]), json!(["BAD=KEY"]), json!(["A\0B"])] {
            let mut json = binding_json();
            json["cold_if_env_nonempty"] = names;
            assert!(validate(json).is_err());
        }
    }

    #[test]
    fn worker_configuration_rejects_unknown_fields_and_invalid_identity() {
        let json = json!({
            "protocol":1,"session_id":"session","artifact_id":"artifact",
            "token":"private-token","connect":{"host":"127.0.0.1","port":43210}
        });
        let valid: WorkerConfig = serde_json::from_value(json.clone()).unwrap();
        assert!(valid.validate().is_ok());
        assert!(!format!("{valid:?}").contains("private-token"));
        for (field, value) in [
            ("protocol", json!(2)),
            ("session_id", json!("")),
            ("artifact_id", json!("")),
            ("token", json!("")),
            ("connect", json!({"host":"::1","port":43210})),
            ("connect", json!({"host":"127.0.0.1","port":0})),
            (
                "connect",
                json!({"host":"127.0.0.1","port":43210,"extra":1}),
            ),
            ("unknown", json!(true)),
        ] {
            let mut changed = json.clone();
            changed[field] = value;
            let result = serde_json::from_value::<WorkerConfig>(changed)
                .map_err(|error| error.to_string())
                .and_then(|config| config.validate());
            assert!(result.is_err(), "accepted {field}");
        }
    }
}
