// Desktop entry point. The app lives in app.rs, shared with the Android
// project (apps/android), which builds it as a library for the JNI loader.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

fn main() {
    app::run();
}
