//! The `notchd` binary: the daemon and its CLI (filled in by Phase 0.3 and later).

const USAGE: &str = "\
usage: notchd <command>

  notchd daemon [--ensure]        run the daemon (--ensure: return at once if one runs)
  notchd ipc <method> [args...]   send an IPC method to the daemon, print its reply
  notchd --help                   this text
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return;
    }
    eprint!("{USAGE}");
    std::process::exit(2);
}
