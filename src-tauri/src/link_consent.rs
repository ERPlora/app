//! The person decides before a link moves this device to another business (hub#2644).

/// What the app asks before a link links another hub: the business the link opens and the one the
/// device is linked to now, if any. Hosts, as the person reads them in the address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub to: String,
    pub from: Option<String>,
}

/// The words of the question in one language: the same type the Android dialog receives.
pub use tauri_plugin_erplora_android::Wording;

/// What the person answered: `true` is «Open». Called once, from whatever thread the dialog
/// answers on.
pub type Answer = Box<dyn FnOnce(bool) + Send>;

/// Who asks: the system's own dialog in the app ([`native`]); the tests put a scripted person here.
/// Managed state, so a link that arrives with nobody to ask is refused, never followed.
pub struct AskBeforeLinking(Box<dyn Fn(Question, Answer) + Send + Sync>);

impl AskBeforeLinking {
    pub fn new(ask: impl Fn(Question, Answer) + Send + Sync + 'static) -> Self {
        Self(Box::new(ask))
    }

    pub fn ask(&self, question: Question, answer: Answer) {
        (self.0)(question, answer)
    }
}

/// The system's own dialog, the one the app asks with. Never the page: the page the window shows
/// may be the very one that sent the link.
pub fn native<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> AskBeforeLinking {
    AskBeforeLinking::new(move |question, answer| ask_the_system(&app, &question, answer))
}

/// Desktop: a modal message box over the main window, «Cancel» (the default) and «Open». Shown from the main
/// thread (macOS requires it) and awaited on another, so the window keeps painting meanwhile. If it
/// cannot be shown, `answer` is dropped unanswered: the link is not followed.
#[cfg(desktop)]
fn ask_the_system<R: tauri::Runtime>(app: &tauri::AppHandle<R>, question: &Question, answer: Answer) {
    use tauri::Manager;
    let words = wording(question, speaks_spanish(sys_locale::get_locale().as_deref()));
    let handle = app.clone();
    let shown = app.run_on_main_thread(move || {
        let mut dialog = rfd::AsyncMessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(&words.title)
            .set_description(&words.message)
            .set_buttons(buttons(&words));
        if let Some(window) = handle.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        let shown = dialog.show();
        std::thread::spawn(move || {
            let result = tauri::async_runtime::block_on(shown);
            answer(said_open(&result, &words.open));
        });
    });
    if let Err(e) = shown {
        log::warn!("shell: could not ask before linking {}: {e}", question.to);
    }
}

/// The two buttons of the desktop dialog, «Cancel» first: the first one is the default (Return on
/// macOS and Windows), and a till's barcode scanner types a Return after every code it reads.
#[cfg(desktop)]
fn buttons(words: &Wording) -> rfd::MessageButtons {
    rfd::MessageButtons::OkCancelCustom(words.cancel.clone(), words.open.clone())
}

/// Whether the desktop dialog came back as «Open». macOS, Windows and GTK all answer a custom pair
/// with the label of the button pressed; anything else (closing it) is a no.
#[cfg(desktop)]
fn said_open(result: &rfd::MessageDialogResult, open: &str) -> bool {
    matches!(result, rfd::MessageDialogResult::Custom(label) if label == open)
}

/// Android: an `AlertDialog` from the app's own plugin, in the device's language. The call blocks
/// until the person answers, so it runs on a thread of its own.
#[cfg(target_os = "android")]
fn ask_the_system<R: tauri::Runtime>(app: &tauri::AppHandle<R>, question: &Question, answer: Answer) {
    use tauri_plugin_erplora_android::ErploraAndroidExt;
    let en = wording(question, false);
    let es = wording(question, true);
    let handle = app.clone();
    let to = question.to.clone();
    std::thread::spawn(move || match handle.erplora_android().ask_to_open_hub(&en, &es) {
        Ok(open) => answer(open),
        Err(e) => {
            log::warn!("shell: could not ask before linking {to}: {e}");
            answer(false);
        }
    });
}

/// iOS is not built (see `WORKFLOW.md`): with no dialog of ours there, a link to another hub is a no.
#[cfg(all(mobile, not(target_os = "android")))]
fn ask_the_system<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, question: &Question, answer: Answer) {
    log::warn!("shell: no dialog to ask before linking {} on this system", question.to);
    answer(false);
}

/// The host of a hub origin as the person reads it in the address (`panaderia.a.erplora.com`, or
/// `127.0.0.1:5173`).
pub fn host_of(origin: &str) -> String {
    let Ok(url) = origin.parse::<tauri::Url>() else {
        return origin.to_string();
    };
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        (None, _) => origin.to_string(),
    }
}

/// The question in English (the source) or in its Spanish translation (ADR-0055/0199). It says what
/// the person gives away by answering yes: the hardware, and the business the app opens from then on.
pub fn wording(question: &Question, spanish: bool) -> Wording {
    let to = &question.to;
    if spanish {
        let message = match &question.from {
            Some(from) => format!(
                "Este dispositivo está enlazado a {from}. Si abres {to}, a partir de ahora usará la impresora, el cajón y el lector de tarjetas de este dispositivo, y la aplicación lo abrirá cada vez que arranque."
            ),
            None => format!(
                "Si abres {to}, usará la impresora, el cajón y el lector de tarjetas de este dispositivo, y la aplicación lo abrirá cada vez que arranque."
            ),
        };
        return Wording {
            title: format!("¿Abrir {to} en este dispositivo?"),
            message,
            open: "Abrir".into(),
            cancel: "Cancelar".into(),
        };
    }
    let message = match &question.from {
        Some(from) => format!(
            "This device is linked to {from}. If you open {to}, it will use this device's printer, cash drawer and card reader from now on, and the app will open it every time it starts."
        ),
        None => format!(
            "If you open {to}, it will use this device's printer, cash drawer and card reader, and the app will open it every time it starts."
        ),
    };
    Wording {
        title: format!("Open {to} on this device?"),
        message,
        open: "Open".into(),
        cancel: "Cancel".into(),
    }
}

/// Spanish unless the device says otherwise: the rule of the bundled offline page and of the Play
/// refusal notice. `locale` is the system's (`es-ES`, `en_US`…), or none when it cannot be read.
/// Desktop only: on Android the plugin picks the language itself.
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn speaks_spanish(locale: Option<&str>) -> bool {
    match locale {
        None => true,
        Some(tag) => tag
            .split(['-', '_'])
            .next()
            .is_some_and(|language| language.eq_ignore_ascii_case("es")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_another(from: Option<&str>) -> Question {
        Question {
            to: "otronegocio.a.erplora.com".into(),
            from: from.map(str::to_string),
        }
    }

    #[test]
    fn the_question_names_the_business_the_link_opens_and_the_one_it_replaces() {
        let asked = wording(&to_another(Some("panaderia.a.erplora.com")), false);
        assert_eq!(asked.title, "Open otronegocio.a.erplora.com on this device?");
        assert_eq!(
            asked.message,
            "This device is linked to panaderia.a.erplora.com. If you open otronegocio.a.erplora.com, it will use \
             this device's printer, cash drawer and card reader from now on, and the app will open it every time \
             it starts."
        );
        assert_eq!(asked.open, "Open");
        assert_eq!(asked.cancel, "Cancel");
    }

    #[test]
    fn the_question_speaks_spanish() {
        let asked = wording(&to_another(Some("panaderia.a.erplora.com")), true);
        assert_eq!(asked.title, "¿Abrir otronegocio.a.erplora.com en este dispositivo?");
        assert_eq!(
            asked.message,
            "Este dispositivo está enlazado a panaderia.a.erplora.com. Si abres otronegocio.a.erplora.com, a \
             partir de ahora usará la impresora, el cajón y el lector de tarjetas de este dispositivo, y la \
             aplicación lo abrirá cada vez que arranque."
        );
        assert_eq!(asked.open, "Abrir");
        assert_eq!(asked.cancel, "Cancelar");
    }

    #[test]
    fn a_device_linked_to_nothing_is_not_told_it_is_linked() {
        let en = wording(&to_another(None), false);
        assert_eq!(
            en.message,
            "If you open otronegocio.a.erplora.com, it will use this device's printer, cash drawer and card \
             reader, and the app will open it every time it starts."
        );
        let es = wording(&to_another(None), true);
        assert_eq!(
            es.message,
            "Si abres otronegocio.a.erplora.com, usará la impresora, el cajón y el lector de tarjetas de este \
             dispositivo, y la aplicación lo abrirá cada vez que arranque."
        );
    }

    #[cfg(desktop)]
    #[test]
    fn only_the_open_button_opens() {
        use rfd::MessageDialogResult as Said;
        assert!(said_open(&Said::Custom("Abrir".into()), "Abrir"));
        // Every backend answers a custom pair with the label pressed; a bare Ok names no button.
        for no in [Said::Custom("Cancelar".into()), Said::Ok, Said::Cancel, Said::No, Said::Yes] {
            assert!(!said_open(&no, "Abrir"), "{no:?}");
        }
    }

    #[cfg(desktop)]
    #[test]
    fn cancel_is_the_button_that_return_presses() {
        // The first button is the default one (Return on macOS and Windows): a till's barcode
        // scanner types a Return after every code, and it must not hand the device away.
        let words = wording(&to_another(Some("panaderia.a.erplora.com")), true);
        let pair = buttons(&words);
        assert!(
            matches!(&pair, rfd::MessageButtons::OkCancelCustom(first, second) if first == "Cancelar" && second == "Abrir"),
            "{pair:?}"
        );
    }

    #[test]
    fn the_business_is_named_by_the_host_of_its_address() {
        assert_eq!(host_of("https://panaderia.a.erplora.com"), "panaderia.a.erplora.com");
        // A local hub is told apart by its port, or two of them read the same.
        assert_eq!(host_of("http://127.0.0.1:5173"), "127.0.0.1:5173");
    }

    #[test]
    fn spanish_unless_the_device_says_otherwise() {
        // The rule of the bundled offline page and of the Play refusal notice.
        for spanish in [None, Some("es"), Some("es-ES"), Some("ES_es"), Some("es-419")] {
            assert!(speaks_spanish(spanish), "{spanish:?}");
        }
        for other in [Some("en-US"), Some("ca-ES"), Some("fr"), Some("estonian-made-up")] {
            assert!(!speaks_spanish(other), "{other:?}");
        }
    }
}
