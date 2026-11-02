//! The `notchd` binary: the daemon and its CLI.

use notchd_daemon::instance::{Overrides, from_env};
use notchd_daemon::{daemon, ensure, ipc};
use std::process::exit;

const USAGE: &str = "\
usage: notchd [options] <command>

  notchd daemon [--ensure]        run the daemon (--ensure: start it detached unless one runs)
  notchd ipc <method> [args...]   send an IPC method to the daemon, print its reply
  notchd --version                its version
  notchd --help                   this text

options, before the command:
  --instance <name>   instance name (env NOTCHD_INSTANCE, default \"default\")
  --socket <path>     socket path (env NOTCHD_SOCKET; default $XDG_RUNTIME_DIR/notchd/<instance>.sock)
  --config <path>     config file (env NOTCHD_CONFIG; default ~/.config/notchd/config.json)
";

fn die(message: &str, code: i32) -> ! {
    eprintln!("notchd: {message}");
    exit(code)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("notchd {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut o = Overrides::default();
    // Options are recognised only before the command.
    while let Some(flag) = args.first().cloned() {
        if !matches!(flag.as_str(), "--instance" | "--socket" | "--config") {
            break;
        }
        if args.len() < 2 {
            die(&format!("{flag} needs a value"), 2);
        }
        let value = args[1].clone();
        args.drain(..2);
        match flag.as_str() {
            "--instance" => o.instance = Some(value),
            "--socket" => o.socket = Some(value.into()),
            _ => o.config = Some(value.into()),
        }
    }
    let Some(command) = args.first().cloned() else {
        eprint!("{USAGE}");
        exit(2)
    };
    let instance = from_env(&o).unwrap_or_else(|e| die(&e, 2));
    match command.as_str() {
        "daemon" => {
            let result = match args.get(1).map(String::as_str) {
                None => daemon::run(instance),
                Some("--ensure") => ensure::ensure(instance).map(|socket| println!("{socket}")),
                Some(other) => die(&format!("unknown daemon option {other}"), 2),
            };
            if let Err(e) = result {
                die(&e, 1);
            }
        }
        "ipc" => {
            let Some(method) = args.get(1) else {
                die("ipc needs a method", 2)
            };
            match ipc::call(&instance.socket, method, &args[2..]) {
                Ok(reply) => println!("{reply}"),
                Err(e) => die(&e, 1),
            }
        }
        _ => {
            eprint!("{USAGE}");
            exit(2)
        }
    }
}
