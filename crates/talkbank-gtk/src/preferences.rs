//! The preferences dialog.

use adw::prelude::*;

use crate::config::{self, Language, Theme};
use crate::i18n::t;
use crate::window::App;

pub fn show(app: &App) {
    let dlg = adw::PreferencesDialog::new();
    let page = adw::PreferencesPage::new();
    page.set_title(&t("Preferences"));
    page.set_icon_name(Some("preferences-system-symbolic"));

    page.add(&appearance_group(app));
    page.add(&behaviour_group(app));
    page.add(&account_group(app));
    page.add(&folders_group(app));

    dlg.add(&page);
    dlg.present(Some(app.window()));
}

fn combo(title: &str, options: &[String], selected: u32) -> (adw::ComboRow, gtk::StringList) {
    let model = gtk::StringList::new(&[]);
    for o in options {
        model.append(o);
    }
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_model(Some(&model));
    row.set_selected(selected);
    (row, model)
}

fn appearance_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Appearance"));

    let (lang_row, _) = combo(
        &t("Language"),
        &[t("Same as the system"), "Italiano".into(), "English".into()],
        config::with(|c| c.language.index()),
    );
    let a = app.clone();
    lang_row.connect_selected_notify(move |r| {
        let chosen = Language::from_index(r.selected());
        if config::with(|c| c.language == chosen) {
            return;
        }
        config::update(|c| c.language = chosen.clone());
        // glibc reads LANGUAGE once at start-up: changing it now would have no
        // effect, so we say so rather than pretend.
        a.show_toast(adw::Toast::new(&t(
            "The language will be applied next time you start TalkBank.",
        )));
    });
    g.add(&lang_row);

    let (theme_row, _) = combo(
        &t("Theme"),
        &[t("Same as the system"), t("Light"), t("Dark")],
        config::with(|c| c.theme.index()),
    );
    let a = app.clone();
    theme_row.connect_selected_notify(move |r| {
        config::update(|c| c.theme = Theme::from_index(r.selected()));
        a.refresh_appearance();
    });
    g.add(&theme_row);

    let font = adw::SpinRow::with_range(7.0, 22.0, 1.0);
    font.set_title(&t("Output text size"));
    font.set_value(config::with(|c| c.font_size) as f64);
    let a = app.clone();
    font.connect_value_notify(move |s| {
        config::update(|c| c.font_size = s.value() as i32);
        a.refresh_appearance();
    });
    g.add(&font);
    g
}

fn behaviour_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Behaviour"));

    let rows: Vec<(String, String, fn(&mut config::Config) -> &mut bool)> = vec![
        (
            t("Show the command line"),
            t("The line you could type in a terminal to get the same result"),
            |c| &mut c.show_command,
        ),
        (
            t("Warn before running"),
            t("Check that the files have what the analysis needs"),
            |c| &mut c.preflight,
        ),
        (t("Include repetitions by default"), "+r6".into(), |c| {
            &mut c.default_r6
        }),
        (t("Save results to a file by default"), "+f".into(), |c| {
            &mut c.default_save
        }),
    ];

    for (title, sub, field) in rows {
        let r = adw::SwitchRow::new();
        r.set_title(&title);
        r.set_subtitle(&sub);
        r.set_active(config::with(|c| {
            // `field` prende &mut, ma qui leggiamo soltanto: una copia basta.
            let mut copy = c.clone();
            *field(&mut copy)
        }));
        let a = app.clone();
        r.connect_active_notify(move |s| {
            let active = s.is_active();
            config::update(|c| *field(c) = active);
            a.refresh_appearance();
        });
        g.add(&r);
    }
    g
}

/// Accesso a TalkBank.
///
/// The catalogue browses without an account; one is only needed to download. We
/// say so in the description, so that anyone who just wants to look does not
/// think they have to register.
fn account_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("TalkBank account"));
    g.set_description(Some(&t(
        "Needed only to download data: browsing the catalogue is free. An account is free too — register on talkbank.org with an email address.",
    )));

    let email = adw::EntryRow::new();
    email.set_title(&t("Email"));
    let saved_email = config::with(|c| c.email.clone()).unwrap_or_default();
    email.set_text(&saved_email);

    let password = adw::PasswordEntryRow::new();
    password.set_title(&t("Password"));
    if !saved_email.is_empty() {
        if let Some(p) = crate::net::load_password(&saved_email) {
            password.set_text(&p);
        }
    }
    g.add(&email);
    g.add(&password);

    let status = adw::ActionRow::new();
    status.set_title(&t("Test connection"));
    status.set_subtitle(&t("Checks the credentials against talkbank.org"));
    let spinner = adw::Spinner::new();
    spinner.set_visible(false);
    status.add_suffix(&spinner);
    let test = gtk::Button::with_label(&t("Test"));
    test.set_valign(gtk::Align::Center);
    status.add_suffix(&test);
    g.add(&status);

    let a = app.clone();
    let e2 = email.clone();
    let p2 = password.clone();
    let st = status.clone();
    let sp = spinner.clone();
    let btn = test.clone();
    test.connect_clicked(move |_| {
        let mail = e2.text().trim().to_string();
        let pass = p2.text().to_string();
        if mail.is_empty() || pass.is_empty() {
            st.set_subtitle(&t("Fill in both fields first."));
            return;
        }
        btn.set_sensitive(false);
        sp.set_visible(true);
        st.set_subtitle(&t("Connecting…"));

        let st2 = st.clone();
        let sp2 = sp.clone();
        let btn2 = btn.clone();
        let a2 = a.clone();
        let mail2 = mail.clone();
        let pass2 = pass.clone();
        crate::net::net().spawn(
            async move { crate::net::net().client().login(&mail2, &pass2).await },
            move |res| {
                sp2.set_visible(false);
                btn2.set_sensitive(true);
                use talkbank_archive::api::LoginOutcome as L;
                let msg = match res {
                    Ok(L::Success) => {
                        config::update(|c| c.email = Some(mail.clone()));
                        // A queue paused because the session had expired picks up
                        // where it was, without replanning; and the archive stops
                        // believing it is signed out.
                        a2.downloads().resume();
                        a2.recheck_archive_login();
                        if let Err(e) = crate::net::store_password(&mail, &pass) {
                            // With no keyring we do not fall back to
                            // settings.ini: a plaintext password on disk is not
                            // an acceptable second best. We keep it for this
                            // session only.
                            tracing::warn!("keyring unavailable: {e}");
                            t("Signed in. This system has no password store, so the password is kept only until you close TalkBank.")
                        } else {
                            t("Signed in. The session lasts about a day.")
                        }
                    }
                    Ok(L::WrongCredentials) => t("Email or password not recognised."),
                    Ok(L::EmailNotVerified) => {
                        t("This account has not been confirmed yet. Check your inbox.")
                    }
                    Ok(L::Other(code)) => {
                        t("TalkBank answered: %s").replace("%s", &code)
                    }
                    Err(e) => e.to_string(),
                };
                st2.set_subtitle(&msg);
                a2.refresh_appearance();
            },
        );
    });

    let forget = adw::ActionRow::new();
    forget.set_title(&t("Sign out"));
    forget.set_subtitle(&t("Forgets the saved password and closes the session"));
    let out = gtk::Button::with_label(&t("Sign out"));
    out.set_valign(gtk::Align::Center);
    out.add_css_class("destructive-action");
    let e3 = email.clone();
    let p3 = password.clone();
    out.connect_clicked(move |_| {
        let mail = e3.text().trim().to_string();
        if !mail.is_empty() {
            crate::net::forget_password(&mail);
        }
        p3.set_text("");
        crate::net::net().spawn(async { crate::net::net().client().logout().await }, |_| {});
    });
    forget.add_suffix(&out);
    g.add(&forget);
    g
}

fn folders_group(app: &App) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&t("Folders"));

    let wd = adw::ActionRow::new();
    wd.set_title(&t("Working folder"));
    // The live folder, not the saved one: if it came from the command line the
    // two differ, and showing the wrong one is confusing.
    wd.set_subtitle(&app.workdir().display().to_string());
    wd.set_subtitle_selectable(true);
    g.add(&wd);

    let bin = adw::ActionRow::new();
    bin.set_title(&t("CLAN programs"));
    bin.set_subtitle(
        &talkbank_engine::find_bin_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| t("not found")),
    );
    bin.set_subtitle_selectable(true);
    g.add(&bin);

    let dl = adw::ActionRow::new();
    dl.set_title(&t("Downloaded corpora"));
    dl.set_subtitle(
        &config::with(|c| c.download_dir.clone())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| t("chosen when you download the first corpus")),
    );
    dl.set_subtitle_selectable(true);
    g.add(&dl);

    g
}
