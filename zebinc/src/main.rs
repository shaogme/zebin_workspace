use clap::{Parser, Subcommand};
use pest::Parser as _;
use pest_derive::Parser as PestParser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "zebinc")]
#[command(about = "Zebin IDL Compiler", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a .zebin file to Rust code
    Compile {
        /// The .zebin file to compile
        input: PathBuf,
        /// Output directory
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
    },
}

#[derive(PestParser)]
#[grammar = "zebin.pest"]
pub struct ZebinParser;

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Compile { input, .. } => {
            println!("Compiling {:?}...", input);
            let content = std::fs::read_to_string(&input).expect("Failed to read input file");
            let file = ZebinParser::parse(Rule::schema, &content)
                .expect("Failed to parse .zebin file")
                .next()
                .unwrap();

            for record in file.into_inner() {
                if record.as_rule() == Rule::struct_def {
                    let mut inner = record.into_inner();
                    let name = inner.next().unwrap().as_str();
                    println!("Found struct: {}", name);
                }
            }
        }
    }
}
