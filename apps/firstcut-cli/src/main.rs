//! Headless entry point for the command registry (docs/v2-plan.md §4.3):
//! `firstcut-cli run <command> key=value …` chains commands and prints one JSON
//! result per command; `mcp` exposes the registry to MCP clients. The convenience
//! subcommands from v1 (`scan`, `verify`, `contact-sheet`, `bench`) return as
//! commands the registry can reach. At M0 the registry itself lands with the
//! engine; this binary only reports that it is a stub and exits 0 so CI can link it.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => print_help(),
        Some("--help" | "help" | "-h") => print_help(),
        Some("run") | Some("mcp") => {
            // The command registry lands with firstcut-engine in M1
            // (docs/v2-plan.md §6, M1); nothing user-visible dispatches until then.
            eprintln!("note: the command registry (`run`, `mcp`) lands in M1");
        }
        Some(other) => {
            eprintln!("firstcut-cli: unknown subcommand `{other}` (try `--help`)");
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "firstcut-cli — command registry runner (v2 rewrite, M0 scaffold)

USAGE:
    firstcut-cli [SUBCOMMAND]

SUBCOMMANDS:
    run     Chain commands through the registry (lands in M1)
    mcp     Expose the registry as MCP tools (lands in M1)
    help    Print this help"
    );
}
