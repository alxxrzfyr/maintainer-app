//! Opt-in startup observations sent only to an explicitly inherited pipe.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

struct Probe {
    output: OwnedFd,
    seen: AtomicU8,
}

static PROBE: OnceLock<Option<Probe>> = OnceLock::new();

fn parse_fd(value: &str) -> Option<i32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<i32>().ok().filter(|fd| *fd >= 3)
}

impl Probe {
    fn from_fd(fd: i32) -> Option<Self> {
        // SAFETY: fcntl validates the borrowed descriptor and returns a new
        // descriptor whose ownership is transferred exactly once to OwnedFd.
        let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicated < 0 {
            return None;
        }
        // SAFETY: duplicated is a newly owned descriptor returned by fcntl.
        let output = unsafe { OwnedFd::from_raw_fd(duplicated) };
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: metadata points to writable storage of the required size.
        if unsafe { libc::fstat(output.as_raw_fd(), metadata.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: successful fstat initialized metadata.
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT != libc::S_IFIFO {
            return None;
        }
        // SAFETY: output is held open; no pointer arguments are passed.
        let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 || flags & libc::O_ACCMODE == libc::O_RDONLY {
            return None;
        }
        // O_NONBLOCK affects the inherited open-file description too. The
        // measurement parent dedicates this pipe to startup observations.
        // SAFETY: output is a valid descriptor and flags are from F_GETFL.
        if unsafe { libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return None;
        }
        // Keep the original inherited descriptor out of subsequently executed
        // collectors too; duplicating with CLOEXEC does not change that flag.
        // SAFETY: fd remains borrowed and fcntl validates it without pointers.
        let descriptor_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if descriptor_flags < 0
            // SAFETY: fd is the original inherited descriptor, still open.
            || unsafe { libc::fcntl(fd, libc::F_SETFD, descriptor_flags | libc::FD_CLOEXEC) } < 0
        {
            return None;
        }
        Some(Self {
            output,
            seen: AtomicU8::new(0),
        })
    }

    fn mark(&self, event: &str) {
        let Some(message) = encode_event(event) else {
            return;
        };
        // SAFETY: the byte slice remains live for this nonblocking write. The
        // bounded record fits PIPE_BUF, so it is written atomically or dropped.
        // Rust's executable runtime ignores SIGPIPE; a closed reader is an error.
        let _ = unsafe {
            libc::write(
                self.output.as_raw_fd(),
                message.as_ptr().cast(),
                message.len(),
            )
        };
    }

    fn mark_once(&self, event: &str) {
        let bit = match event {
            "process-start" => 1,
            "mapped" => 2,
            "usable" => 4,
            "first-data" => 8,
            event if valid_renderer(event) => 16,
            _ => return,
        };
        if self.seen.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
            self.mark(event);
        }
    }
}

fn probe() -> Option<&'static Probe> {
    PROBE
        .get_or_init(|| {
            let value = std::env::var("MAINTAINER_PROBE_FD").ok()?;
            Probe::from_fd(parse_fd(&value)?)
        })
        .as_ref()
}

pub fn enabled() -> bool {
    probe().is_some()
}

pub fn mark(event: &str) {
    if let Some(probe) = probe() {
        probe.mark(event);
    }
}

pub fn mark_once(event: &str) {
    if let Some(probe) = probe() {
        probe.mark_once(event);
    }
}

pub fn requested_page() -> &'static str {
    match std::env::var("MAINTAINER_PROBE_PAGE").as_deref() {
        Ok("System") => "System",
        Ok("Processes") => "Processes",
        Ok("Network") => "Network",
        _ => "Overview",
    }
}

fn valid_renderer(event: &str) -> bool {
    event.strip_prefix("renderer:").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 96
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

fn encode_event(event: &str) -> Option<String> {
    if !matches!(
        event,
        "process-start"
            | "mapped"
            | "usable"
            | "first-data"
            | "close-requested"
            | "window-destroyed"
            | "application-shutdown"
    ) && !valid_renderer(event)
    {
        return None;
    }
    let mut timestamp = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: timestamp is writable storage and CLOCK_MONOTONIC is fixed.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, timestamp.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful clock_gettime initialized timestamp.
    let timestamp = unsafe { timestamp.assume_init() };
    let nanos = u128::try_from(timestamp.tv_sec)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u128::try_from(timestamp.tv_nsec).ok()?)?;
    let record = format!("{{\"event\":\"{event}\",\"monotonic_ns\":{nanos}}}\n");
    (record.len() <= 256).then_some(record)
}

/// Exercise the existing navigation signal path across two main-loop turns.
/// Mapping and this canary do not establish compositor presentation.
pub fn install_navigation_canary(
    window: &gtk4::ApplicationWindow,
    stack: &gtk4::Stack,
    sidebar: &gtk4::Box,
) {
    use gtk4::prelude::*;
    if !enabled() {
        return;
    }
    let window_weak = window.downgrade();
    let stack_weak = stack.downgrade();
    let sidebar_weak = sidebar.downgrade();
    let scheduled = std::cell::Cell::new(false);
    window.connect_map(move |_| {
        if scheduled.replace(true) {
            return;
        }
        let window_weak = window_weak.clone();
        let stack_weak = stack_weak.clone();
        let sidebar_weak = sidebar_weak.clone();
        gtk4::glib::idle_add_local_once(move || {
            let (Some(window), Some(stack), Some(sidebar)) = (
                window_weak.upgrade(),
                stack_weak.upgrade(),
                sidebar_weak.upgrade(),
            ) else {
                return;
            };
            let target = requested_page();
            let canary = if target == "Overview" {
                "System"
            } else {
                "Overview"
            };
            if !window.is_mapped() || stack.child_by_name(target).is_none() {
                return;
            }
            let Some(button) = navigation_button(sidebar.upcast_ref(), canary) else {
                return;
            };
            button.emit_clicked();
            if stack.visible_child_name().as_deref() != Some(canary) {
                return;
            }
            gtk4::glib::idle_add_local_once(move || {
                let (Some(window), Some(stack), Some(sidebar)) = (
                    window_weak.upgrade(),
                    stack_weak.upgrade(),
                    sidebar_weak.upgrade(),
                ) else {
                    return;
                };
                let Some(button) = navigation_button(sidebar.upcast_ref(), target) else {
                    return;
                };
                button.emit_clicked();
                if window.is_mapped()
                    && stack.visible_child_name().as_deref() == Some(target)
                    && stack
                        .visible_child()
                        .is_some_and(|page| page.first_child().is_some())
                {
                    mark_once("usable");
                }
            });
        });
    });
}

fn navigation_button(widget: &gtk4::Widget, page: &str) -> Option<gtk4::Button> {
    use gtk4::prelude::*;
    if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
        if button.has_css_class("maintainer_nav_button")
            && button.tooltip_text().as_deref() == Some(page)
        {
            return Some(button.clone());
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = navigation_button(&widget, page) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn descriptor_and_event_inputs_are_bounded_and_allowlisted() {
        assert_eq!(parse_fd("12"), Some(12));
        for invalid in ["", "2", "-1", " 3", "3\n", "9999999999999999999"] {
            assert_eq!(parse_fd(invalid), None);
        }
        assert!(encode_event("renderer:GskGLRenderer").unwrap().len() <= 256);
        assert!(encode_event("renderer:bad\"name").is_none());
        assert!(encode_event("private-user-input").is_none());
        for event in [
            "close-requested",
            "window-destroyed",
            "application-shutdown",
        ] {
            assert!(encode_event(event).is_some());
        }
        assert!(Probe::from_fd(-1).is_none());
    }

    #[test]
    fn inherited_pipe_is_nonblocking_and_emits_once_without_extra_fields() {
        let mut descriptors = [-1; 2];
        // SAFETY: descriptors provides space for the two returned descriptors.
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        // SAFETY: each descriptor from pipe2 is newly owned, used exactly once.
        let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        // SAFETY: the second descriptor is distinct from reader.
        let writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        let probe = Probe::from_fd(writer.as_raw_fd()).unwrap();
        // SAFETY: probe holds a live output descriptor.
        assert_ne!(
            unsafe { libc::fcntl(probe.output.as_raw_fd(), libc::F_GETFL) } & libc::O_NONBLOCK,
            0
        );
        // SAFETY: probe holds a live output descriptor.
        assert_ne!(
            unsafe { libc::fcntl(probe.output.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        probe.mark_once("mapped");
        probe.mark_once("mapped");
        drop(probe);
        drop(writer);
        let mut text = String::new();
        std::fs::File::from(reader)
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with("{\"event\":\"mapped\",\"monotonic_ns\":"));
        let nanos = text
            .trim()
            .strip_suffix('}')
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap();
        assert!(nanos.parse::<u128>().unwrap() > 0);
        assert!(text.len() <= 256);
    }
}
