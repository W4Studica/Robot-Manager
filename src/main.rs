use robot_manager::config::{self, Config};
use robot_manager::manager::{Manager, Policy};
use robot_manager::output::Journal;
use robot_manager::riolog::RioLogServer;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

fn main() {
    if let Err(error) = run() {
        robot_manager::log("ERROR", error);
        std::process::exit(1);
    }
}
fn run() -> io::Result<()> {
    if let Some(result) = robot_manager::network::cli::dispatch_os(
        &std::env::args_os().collect::<Vec<_>>(),
        &robot_manager::network::Network::default(),
        &mut robot_manager::network::system::Linux,
    ) {
        use std::io::Write;
        io::stdout().write_all(result?.as_bytes())?;
        return Ok(());
    }
    let mut path = PathBuf::from(config::DEFAULT_CONFIG);
    let mut check = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--config") => {
                path = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| io::Error::other("--config requires a path"))?
            }
            Some(value) if value.starts_with("--config=") => {
                path = value["--config=".len()..].into()
            }
            Some("--check-config") => check = true,
            Some("--help" | "-h") => {
                println!("robot_manager [--config PATH] [--check-config]\nRun and supervise the VMX robot program.\nNetworking: robot_manager network --help\nDefault config: {}", config::DEFAULT_CONFIG);
                return Ok(());
            }
            Some("--version" | "-V") => {
                println!("robot_manager {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                return Err(io::Error::other(format!(
                    "unknown argument: {}",
                    arg.to_string_lossy()
                )))
            }
        }
    }
    let config = Config::load(&path)?;
    if check {
        println!("configuration valid: {}", path.display());
        return Ok(());
    }
    let supervisor = robot_manager::realtime::configure_supervisor()?;
    robot_manager::log(
        "INFO",
        format_args!(
            "supervisor scheduling: {}/{}",
            supervisor.name(),
            supervisor.priority
        ),
    );
    let stopping = Arc::new(AtomicBool::new(false));
    // Signal callbacks only set an atomic flag; shutdown and waitpid run in the
    // supervisor's normal control flow, never inside a signal handler.
    signal_hook::flag::register(libc::SIGTERM, stopping.clone())?;
    signal_hook::flag::register(libc::SIGINT, stopping.clone())?;
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")?;
    let rt_kernel = robot_manager::realtime::kernel_rt(std::path::Path::new("/"));
    let kernel_state = match rt_kernel {
        Some(true) => "detected",
        Some(false) => "not detected",
        None => "unknown",
    };
    let requested = config.realtime.target();
    robot_manager::log("INFO", format_args!(
        "PREEMPT_RT kernel: {kernel_state}; robot RT mode: {:?}, requested policy: {}/{}; userspace scheduling does not enable PREEMPT_RT",
        config.realtime.mode, requested.name(), requested.priority
    ));
    let has_lse = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| config::cpu_has_lse(&text));
    let env = config.environment(kernel.trim(), std::env::vars().collect(), has_lse)?;
    let mut server = if config.riolog_enabled {
        match RioLogServer::start(config.riolog_port, config.riolog_history) {
            Ok(server) => Some(server),
            Err(error) => {
                robot_manager::log(
                    "ERROR",
                    format_args!("RioLog console could not listen: {error}"),
                );
                None
            }
        }
    } else {
        None
    };
    let manager = Manager {
        command_file: config.command_file(),
        cwd: config.robot_home.clone(),
        env,
        policy: Policy {
            stop_grace: config.stop_grace()?,
            stop_signals: config.signals(),
            realtime: config.realtime,
            ..Policy::default()
        },
        stopping,
        starts: Arc::new(AtomicUsize::new(0)),
        journal: Journal::stdout()?,
        console: server
            .as_ref()
            .map(|s| Arc::new(s.publisher()) as Arc<dyn robot_manager::output::LineSink>),
    };
    robot_manager::log(
        "INFO",
        format_args!(
            "robot_manager starting; command: {}",
            manager.command_file.display()
        ),
    );
    let result = manager.run();
    if let Some(server) = server.as_mut() {
        server.stop();
    }
    result?;
    robot_manager::log("INFO", "robot_manager stopped");
    Ok(())
}
