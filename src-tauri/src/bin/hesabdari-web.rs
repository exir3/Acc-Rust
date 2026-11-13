fn main() {
    if let Err(message) = hesabdari_lib::web::run() {
        eprintln!("[hesabdari-web] {message}");
        std::process::exit(1);
    }
}
