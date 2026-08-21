use gtk4::prelude::*;
use maintainer_app::application;

fn main() {
    let app = application::build_application();
    application::run_application(&app);
    let exit_code = app.run();
    std::process::exit(exit_code.into());
}
