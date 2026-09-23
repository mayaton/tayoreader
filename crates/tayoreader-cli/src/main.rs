use clap::Parser;

#[derive(Parser)]
#[command(name = "tayoreader", version, about = "TayoReader: RSS リーダー")]
struct Cli {}

fn main() {
    Cli::parse();
}
