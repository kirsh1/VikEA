use std::path::Path;

fn main() {
    let code = cc_switch_lib::sidecar::main_entry(Path::new("."));
    std::process::exit(code);
}
