//! The std platform (`crates/vard-lang/src/std.vard`) on desktop operating systems.
//!
//! Preferences and secure values are JSON files in the per-user config folder
//! (`~/Library/Application Support/vard/<app>` on macOS, `$XDG_CONFIG_HOME/vard/<app>` or
//! `~/.config/vard/<app>` on Linux, `%APPDATA%\vard\<app>` on Windows); the secure file is
//! readable by the user only (0600 on Unix). URLs, sharing (clipboard) and notifications go
//! through the system's own commands. Push has no desktop provider: `getPushToken` is nil.

use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use vard_runtime::native::NativeHost;

/// Callback answers the runtime cannot take while it is calling us: delivered after the event.
pub type Answers = Rc<RefCell<Vec<(u64, String)>>>;

pub struct DesktopHost {
    dir: PathBuf,
    prefs: BTreeMap<String, String>,
    secure: BTreeMap<String, String>,
    pub answers: Answers,
}

pub fn config_dir(app: &str) -> PathBuf {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        env("APPDATA").unwrap_or_else(|| PathBuf::from("."))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support")).unwrap_or_else(|| PathBuf::from("."))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config"))).unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("vard").join(app)
}

fn read(path: &PathBuf) -> BTreeMap<String, String> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

impl DesktopHost {
    pub fn new(app: &str) -> DesktopHost {
        let dir = config_dir(app);
        DesktopHost { prefs: read(&dir.join("prefs.json")), secure: read(&dir.join("secure.json")), dir, answers: Rc::default() }
    }

    fn save(&self, name: &str, map: &BTreeMap<String, String>) {
        let _ = std::fs::create_dir_all(&self.dir);
        let path = self.dir.join(name);
        let _ = std::fs::write(&path, serde_json::to_string_pretty(map).unwrap_or_default());
        #[cfg(unix)]
        if name == "secure.json" {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }

    fn answer(&self, cb: &Value, v: Value) {
        if let Some(id) = cb.get("$callback").and_then(Value::as_u64) {
            self.answers.borrow_mut().push((id, v.to_string()));
        }
    }
}

/// Run a system command without waiting for it; false if it could not start.
fn spawn(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

pub fn open_url(url: &str) -> bool {
    if cfg!(target_os = "macos") {
        spawn("open", &[url])
    } else if cfg!(target_os = "windows") {
        spawn("cmd", &["/C", "start", "", url])
    } else {
        spawn("xdg-open", &[url])
    }
}

fn copy(text: &str) -> bool {
    use std::io::Write;
    let tries: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    };
    for (cmd, args) in tries {
        if let Ok(mut child) = Command::new(cmd).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().is_ok_and(|s| s.success());
        }
    }
    false
}

fn notify(title: &str, body: &str) -> bool {
    if cfg!(target_os = "macos") {
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        spawn("osascript", &["-e", &format!("display notification \"{}\" with title \"{}\"", q(body), q(title))])
    } else if cfg!(target_os = "windows") {
        eprintln!("notification: {} — {}", title, body);
        true
    } else {
        spawn("notify-send", &[title, body])
    }
}

impl NativeHost for DesktopHost {
    fn call(&mut self, name: &str, args_json: &str) -> Option<String> {
        let args: Vec<Value> = serde_json::from_str(args_json).unwrap_or_default();
        let s = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let out = match name {
            "prefGet" => self.prefs.get(&s(0)).map(|v| json!(v)).unwrap_or(Value::Null),
            "prefSet" => {
                self.prefs.insert(s(0), s(1));
                self.save("prefs.json", &self.prefs);
                Value::Null
            }
            "prefRemove" => {
                self.prefs.remove(&s(0));
                self.save("prefs.json", &self.prefs);
                Value::Null
            }
            "secureGet" => self.secure.get(&s(0)).map(|v| json!(v)).unwrap_or(Value::Null),
            "secureSet" => {
                self.secure.insert(s(0), s(1));
                self.save("secure.json", &self.secure);
                json!(true)
            }
            "secureRemove" => {
                self.secure.remove(&s(0));
                self.save("secure.json", &self.secure);
                Value::Null
            }
            "openUrl" => {
                open_url(&s(0));
                Value::Null
            }
            "shareText" => {
                copy(&s(0));
                Value::Null
            }
            "haptic" | "openAppSettings" => Value::Null,
            // desktop apps need no runtime grant for notifications; devices are not exposed
            "permissionStatus" => json!(if s(0) == "notifications" { "granted" } else { "denied" }),
            "requestPermission" => {
                let st = if s(0) == "notifications" { "granted" } else { "denied" };
                self.answer(args.get(1).unwrap_or(&Value::Null), json!(st));
                Value::Null
            }
            "showNotification" => {
                notify(&s(0), &s(1));
                Value::Null
            }
            "getPushToken" => {
                self.answer(args.first().unwrap_or(&Value::Null), Value::Null);
                Value::Null
            }
            _ => return None,
        };
        Some(out.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_and_secure_persist() {
        let app = format!("vard-desktop-test-{}", std::process::id());
        let mut h = DesktopHost::new(&app);
        assert_eq!(h.call("prefSet", r#"["k","v"]"#).as_deref(), Some("null"));
        assert_eq!(h.call("secureSet", r#"["t","s3cret"]"#).as_deref(), Some("true"));
        let mut again = DesktopHost::new(&app);
        assert_eq!(again.call("prefGet", r#"["k"]"#).as_deref(), Some("\"v\""));
        assert_eq!(again.call("secureGet", r#"["t"]"#).as_deref(), Some("\"s3cret\""));
        assert_eq!(again.call("requestPermission", r#"["camera",{"$callback":7}]"#).as_deref(), Some("null"));
        assert_eq!(again.answers.borrow().as_slice(), &[(7, "\"denied\"".to_string())]);
        assert_eq!(again.call("nope", "[]"), None);
        let _ = std::fs::remove_dir_all(config_dir(&app));
    }
}
