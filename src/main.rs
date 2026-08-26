mod app;
mod brewfile;
mod catalog;
mod ensure;

use clap::Parser;

use app::Opts;
use ensure::Live;

fn main() {
    let opts = Opts::parse();
    match app::run(&Live, opts) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
