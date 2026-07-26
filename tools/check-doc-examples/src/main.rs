use std::env;
use std::path::Path;

use check_doc_examples::{parse_base_ref, run};

fn main() {
    let argv: Vec<String> = env::args().skip(1).collect();
    let base_ref = parse_base_ref(&argv);
    let cwd = env::current_dir().expect("current directory must be readable");
    std::process::exit(run(Path::new(&cwd), &base_ref));
}
