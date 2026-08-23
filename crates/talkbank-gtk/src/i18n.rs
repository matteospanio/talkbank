//! Translations.
//!
//! We use the system gettext against the `.mo` files compiled from `po/`, so the
//! strings already translated for the C version still apply: the msgids match.
//!
//! The language has to be chosen **before** gettext is initialised, because
//! glibc reads `LANGUAGE` once at start-up. Changing it in the preferences takes
//! effect on the next run, and that is what the interface tells the user.

use std::path::PathBuf;

pub const DOMAIN: &str = "talkbank";

/// Translates a string. The short name is deliberate: it appears everywhere.
pub fn t(s: &str) -> String {
    gettextrs::gettext(s)
}

/// Singular/plural form.
pub fn tn(singular: &str, plural: &str, n: u32) -> String {
    gettextrs::ngettext(singular, plural, n)
}

/// Where the catalogues live: next to the executable when working from the build
/// directory, otherwise wherever the installation put them.
fn locale_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TALKBANK_LOCALE") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // In order: the build directory, an installed prefix
            // (`<prefix>/bin/talkbank` -> `<prefix>/share/locale`), and a couple
            // of layouts that turn up when running from a cargo target dir.
            for rel in [
                "locale",
                "../share/locale",
                "../../build/locale",
                "../build/locale",
            ] {
                let p = parent.join(rel);
                if p.is_dir() {
                    return p;
                }
            }
        }
    }
    PathBuf::from("/usr/local/share/locale")
}

/// Call this first thing in `main`, before anything else that might read the
/// environment.
pub fn init(language_code: &str) {
    if language_code != "auto" {
        // SAFETY: still single-threaded here, before GTK starts.
        unsafe {
            std::env::set_var("LANGUAGE", language_code);
        }
    }
    let _ = gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    let dir = locale_dir();
    if let Err(e) = gettextrs::bindtextdomain(DOMAIN, dir.clone()) {
        tracing::warn!("bindtextdomain({}) failed: {e}", dir.display());
    }
    let _ = gettextrs::bind_textdomain_codeset(DOMAIN, "UTF-8");
    if let Err(e) = gettextrs::textdomain(DOMAIN) {
        tracing::warn!("textdomain failed: {e}");
    }
}
