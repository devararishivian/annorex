fn main() {
    let code = annorex::cli::run(
        std::env::args_os(),
        |key| std::env::var_os(key),
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
