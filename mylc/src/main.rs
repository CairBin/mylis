//! mylc 二进制入口：只负责组装模块并把进程退出码交给驱动。

mod cli;
mod driver;
mod i18n;

fn main() {
    let cmd = match cli::parse() {
        Ok(c) => c,
        Err(m) => {
            eprintln!("mylc: {}", m);
            cli::usage();
            std::process::exit(2);
        }
    };
    let code = match cmd {
        cli::Command::Compile(a) => driver::run(a),
        cli::Command::Build { target, run, update_lock, offline, std, profile } => {
            driver::build(&target, run, update_lock, offline, std, profile)
        }
        cli::Command::Test => driver::test(),
    };
    std::process::exit(code);
}
