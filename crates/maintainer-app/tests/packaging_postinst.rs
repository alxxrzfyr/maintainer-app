//! Exercise installed-metadata refresh without touching the host's services.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "maintainer-postinst-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        for directory in [
            "tools",
            "run/systemd/system",
            "usr/share/glib-2.0/schemas",
            "usr/share/icons/hicolor",
            "usr/share/applications",
        ] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        Self(root)
    }

    fn tool(&self, name: &str, checks: &str) {
        let path = self.0.join("tools").join(name);
        let script = format!(
            "#!/bin/sh\nset -eu\nprintf '{name}' >> \"$FIXTURE_LOG\"\nprintf '\\t%s' \"$@\" >> \"$FIXTURE_LOG\"\nprintf '\\n' >> \"$FIXTURE_LOG\"\n{checks}\n"
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
}

#[test]
fn postinst_refreshes_metadata_without_restarting_polkit_or_active_helpers() {
    let fixture = Fixture::new();
    // Polkit monitors its action directories and invalidates the action cache
    // when policy files change; restarting it can interrupt authorization.
    // Primary source: polkitbackendactionpool.c, dir_monitor_changed(),
    // https://raw.githubusercontent.com/polkit-org/polkit/main/src/polkitbackend/polkitbackendactionpool.c
    // Permit metadata refresh only: any restart/reload/stop of Polkit or an
    // active maintenance helper makes this executed regression fail.
    fixture.tool(
        "systemctl",
        "[ \"$#\" -eq 1 ] && [ \"$1\" = daemon-reload ]",
    );
    fixture.tool(
        "glib-compile-schemas",
        "[ \"$#\" -eq 2 ] && [ \"$1\" = --strict ]",
    );
    fixture.tool("gtk-update-icon-cache", ":");
    fixture.tool("update-desktop-database", ":");

    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/debian/postinst");
    let script = fs::read_to_string(source)
        .unwrap()
        .replace("/run/", &format!("{}/run/", quoted(&fixture.0)))
        .replace("/usr/share/", &format!("{}/usr/share/", quoted(&fixture.0)));
    let path = fixture.0.join("postinst");
    fs::write(&path, script).unwrap();
    let log = fixture.0.join("calls");
    let output = Command::new("/bin/sh")
        .arg(path)
        .arg("configure")
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fixture.0.join("tools").display()),
        )
        .env("FIXTURE_LOG", &log)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "postinst failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(log).unwrap();
    assert!(
        calls.lines().any(|line| line == "systemctl\tdaemon-reload"),
        "{calls}"
    );
    assert!(
        calls.lines().any(|line| line
            == format!(
                "glib-compile-schemas\t--strict\t{}/usr/share/glib-2.0/schemas/",
                fixture.0.display()
            )),
        "{calls}"
    );
    assert!(
        calls
            .lines()
            .filter(|line| line.starts_with("systemctl\t"))
            .all(|line| line == "systemctl\tdaemon-reload"),
        "service lifecycle changed: {calls}"
    );
}
