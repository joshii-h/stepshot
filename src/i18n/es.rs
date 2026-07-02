//! Spanish strings — machine-assisted translation; native review welcome.

use super::Strings;

pub static STRINGS: Strings = Strings {
    html_lang: "es",
    click_left: "Clic izquierdo",
    click_right: "Clic derecho",
    click_middle: "Clic central",
    action_on: "{action} en {element}",
    in_window: "{action} en la ventana «{title}»",
    in_active_window: "{action} en la ventana activa",
    in_screen: "{action} (vista de pantalla completa)",
    element_generic: "elemento",
    tray_ready: "stepshot — listo",
    tray_recording: "● Grabando — {n} paso(s)",
    tt_ready: "Listo",
    tt_recording: "Grabando — {n} paso(s)",
    menu_start: "Iniciar grabación",
    menu_stop: "Detener la grabación y escribir el informe",
    menu_open_folder: "Abrir la última carpeta de informes",
    menu_quit: "Salir de stepshot",
    notify_started: "Grabación iniciada",
    notify_stopped: "Grabación detenida — {n} paso(s). Informe guardado.",
    notify_no_input: "No hay dispositivo de entrada: la captura de clics está desactivada. Añádete al grupo «input» y reinicia el equipo (volver a iniciar sesión no basta).",
    report_heading: "Grabación",
    report_started: "Iniciado: {x}",
    report_total: "Pasos en total: {n}",
    report_step: "Paso {n}",
    report_steps_word: "paso(s)",
    report_self_contained: "autónomo",
};
