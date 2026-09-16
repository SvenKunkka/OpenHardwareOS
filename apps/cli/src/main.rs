//! Headless OpenHardwareOS: `ohm-cli`.
//!
//! The CLI is not a toy. It runs the same runtime and the same automation engine
//! as the desktop app, with a text front end, which makes it the tool for:
//!
//! * verifying an installation — `ohm-cli doctor`,
//! * watching real sensors without a GUI — `ohm-cli watch`,
//! * exercising the whole cooling loop on simulated hardware, on any machine,
//!   with no hardware at all — `ohm-cli demo`,
//! * inspecting the Open Device Protocol flow — `ohm-cli protocol`,
//! * CI — every subcommand is deterministic and exits non-zero on failure.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use ohm_adapter_api::HardwareAdapter;
use ohm_adapter_mock::{ClockMode, LoadProfile, MockAdapter, MockConfig};
use ohm_adapters::{AdapterOptions, build_adapters};
use ohm_automation::examples::suggest_for;
use ohm_automation::{AutomationEngine, Rule, RuleStore, merge_suggestions};
use ohm_core::ConfigPaths;
use ohm_core::logging::LogLevel;
use ohm_device_model::{DeviceState, Value, caps};
use ohm_runtime::{
    ControlRelease, DeviceView, Runtime, ServiceError, ServiceGuard, ServiceOutcome, ServiceState,
    SettingsStore,
};

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Print what the exit path actually achieved, without over-claiming.
///
/// A write the adapter could not read back is *not* a released channel, and a
/// refused or failed channel is a problem the operator has to see — on stderr,
/// so it survives being piped into a log.
fn report_control_release(release: &ControlRelease) {
    if release.skipped_by_config {
        return;
    }
    let problems = release.problems();
    if !problems.is_empty() {
        eprintln!(
            "control release finished with problems: {}",
            release.summary()
        );
        for problem in problems {
            eprintln!("  - {problem}");
        }
    }
    for caveat in release.caveats() {
        eprintln!("note: {caveat}");
    }
}

fn print_header(title: &str) {
    println!();
    println!("{}", "─".repeat(78));
    println!("  {title}");
    println!("{}", "─".repeat(78));
}

/// Render one reading, honouring `unavailable` reasons instead of showing zero.
fn render(state: &Option<DeviceState>, capability: &str, suffix: &str) -> String {
    let Some(state) = state else {
        return "—".to_string();
    };
    match state.get(capability) {
        Some(reading) if reading.is_ok() => {
            let value = reading.value().cloned().unwrap_or(Value::Integer(0));
            format!("{value}{suffix}")
        }
        Some(reading) => format!(
            "[{}]",
            reading
                .reason()
                .unwrap_or(ohm_device_model::UnavailableReason::Unknown)
                .as_str()
        ),
        None => "—".to_string(),
    }
}

fn device_line(view: &DeviceView) -> String {
    let state = &view.state;
    let mut parts = vec![render(state, caps::TEMPERATURE_CORE, " °C")];
    for (capability, label, suffix) in [
        (caps::CPU_LOAD, "load", " %"),
        (caps::GPU_LOAD, "load", " %"),
        (caps::POWER_GPU, "power", " W"),
        (caps::POWER_TOTAL, "power", " W"),
        (caps::FAN_RPM, "", " RPM"),
        (caps::FAN_SPEED_PERCENT, "duty", " %"),
        (caps::DISK_FREE, "free", " B"),
    ] {
        let rendered = render(state, capability, suffix);
        if rendered != "—" {
            parts.push(if label.is_empty() {
                rendered
            } else {
                format!("{label} {rendered}")
            });
        }
    }
    format!(
        "{:<34} {:<10} {}",
        truncate(&view.device.name, 34),
        view.status.as_str(),
        parts.join(" · ")
    )
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn print_adapters(runtime: &Runtime) {
    for view in runtime.adapter_views() {
        let detail = view
            .status
            .detail
            .clone()
            .map(|detail| format!(" — {detail}"))
            .unwrap_or_default();
        println!(
            "  {:<8} {:<12} {:<3} device(s){detail}",
            view.info.id.as_str(),
            view.status.state.as_str(),
            view.status.device_count
        );
    }
}

fn print_devices(runtime: &Runtime) {
    let devices = runtime.devices();
    if devices.is_empty() {
        println!("  no devices yet");
        return;
    }
    for view in &devices {
        println!("  {}", device_line(view));
    }
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Profile {
    /// A quiet machine: nothing to see.
    Idle,
    /// Load cycling between idle and gaming on a 40 s loop.
    Gaming,
    /// A slow sine wave of load.
    Wave,
}

impl Profile {
    fn to_load_profile(self) -> LoadProfile {
        match self {
            Self::Idle => LoadProfile::Constant { load: 0.05 },
            Self::Gaming => LoadProfile::gaming_demo(),
            Self::Wave => LoadProfile::Wave {
                min: 0.1,
                max: 0.95,
                period_ms: 30_000,
                phase: 0.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevelArg {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevelArg> for LogLevel {
    fn from(value: LogLevelArg) -> Self {
        match value {
            LogLevelArg::Error => LogLevel::Error,
            LogLevelArg::Warn => LogLevel::Warn,
            LogLevelArg::Info => LogLevel::Info,
            LogLevelArg::Debug => LogLevel::Debug,
            LogLevelArg::Trace => LogLevel::Trace,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "ohm-cli",
    version,
    about = "OpenHardwareOS headless runtime: discover hardware, read sensors, run cooling rules",
    long_about = "Runs the OpenHardwareOS hardware runtime and automation engine without a GUI.\n\
                  `demo` exercises the full cooling loop on simulated hardware; `doctor` shows\n\
                  what a real machine exposes and whether it can be controlled."
)]
struct Cli {
    /// Configuration directory (defaults to the platform location).
    #[arg(long, global = true, env = "OHM_CONFIG_DIR")]
    config_dir: Option<String>,

    /// Log level.
    #[arg(long, global = true, value_enum, default_value = "warn")]
    log_level: LogLevelArg,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// What hardware is attached, and can it be controlled?
    Doctor {
        /// Also register the simulated providers.
        #[arg(long)]
        mock: bool,
        /// Print one JSON object instead of the human-readable report: every
        /// provider with its status and reason, every device with its readings and
        /// the reason for each missing one, and the capability notes. Nothing else
        /// is written, so a script can read it directly — and so a reading can be
        /// compared with the platform's own source by something other than my eyes.
        #[arg(long)]
        json: bool,
    },
    /// Print the current readings once.
    Status {
        #[arg(long)]
        mock: bool,
        /// Print the runtime snapshot as JSON, and nothing else.
        #[arg(long)]
        json: bool,
    },
    /// Watch live values until interrupted.
    Watch {
        #[arg(long)]
        mock: bool,
        /// Seconds between refreshes.
        #[arg(long, default_value_t = 1.0)]
        interval: f64,
        /// Stop after this many refreshes (0 = forever).
        #[arg(long, default_value_t = 0)]
        count: u32,
    },
    /// Run the closed loop on simulated hardware: temperature -> rule -> fan.
    Demo {
        #[arg(long, value_enum, default_value = "gaming")]
        profile: Profile,
        /// Simulated seconds per step.
        #[arg(long, default_value_t = 1.0)]
        step_seconds: f64,
        /// Number of steps.
        #[arg(long, default_value_t = 90)]
        steps: u32,
        /// Room temperature for the simulation.
        #[arg(long, default_value_t = 25.0)]
        ambient: f64,
    },
    /// Inspect and edit automation rules.
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },
    /// Show the audit trail (every hardware write is recorded).
    Audit {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Channels a rule left behind, and whether they were made safe.
    Handovers {
        /// Re-arm handovers that ran out of attempts, then report.
        #[arg(long)]
        retry: bool,
    },
    /// Where OpenHardwareOS keeps its files.
    Paths,
    /// Show an Open Device Protocol exchange with the simulated OpenFan.
    Protocol,
    /// Run the automation rules as a background service, or ask about one.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Collect everything needed to judge this machine, into one file to send back.
    ///
    /// Two people need this: whoever is validating the hardware on a real machine,
    /// and whoever reads the result afterwards. It gathers what *we* report and, where
    /// the platform has one, the platform's own answer to the same question — the raw
    /// `hwmon` files, `/proc/meminfo`, `df` — so a reading can be checked without
    /// being taken on trust, and so a disagreement can be seen by somebody who is not
    /// sitting at that machine.
    Report {
        /// Where to write it. Defaults to a timestamped file in the config directory's
        /// `reports/`.
        #[arg(long)]
        out: Option<String>,
        /// Also register the simulated providers, for a dry run of the collector.
        #[arg(long)]
        mock: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceAction {
    /// Run the rules until interrupted (Ctrl-C, or SIGTERM from a service manager).
    ///
    /// This is the process that keeps cooling decisions running when no window is
    /// open. It holds a state file while it runs, so a second service — or the
    /// desktop application — can see that something else already owns the channels;
    /// on exit it releases control exactly as the desktop does, and says what the
    /// release achieved.
    Run {
        #[arg(long)]
        mock: bool,
        /// Milliseconds between heartbeats to the state file.
        #[arg(long, default_value_t = 1000)]
        heartbeat_ms: u64,
        /// Stop after this many automation cycles (0 = until interrupted).
        #[arg(long, default_value_t = 0)]
        max_ticks: u64,
    },
    /// Is a service running here? Exits 1 when it is not.
    Status,
}

#[derive(Debug, Subcommand)]
enum RulesAction {
    /// List rules with their live status.
    List,
    /// Install starter rules for the attached hardware.
    Suggest {
        #[arg(long)]
        mock: bool,
    },
    /// Delete a rule by id.
    Delete { id: String },
    /// Enable or disable a rule: `rules set-enabled gpu-cooling false`.
    SetEnabled {
        id: String,
        /// `true` or `false`. Given explicitly so it cannot be mistaken for a flag.
        #[arg(action = clap::ArgAction::Set, value_parser = clap::value_parser!(bool))]
        enabled: bool,
    },
    /// Validate a rule file without running it.
    Check { path: String },
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

struct Session {
    runtime: Runtime,
    engine: AutomationEngine,
    paths: ConfigPaths,
}

impl Session {
    /// Open the runtime against the configured providers.
    ///
    /// `use_mock` also enables simulated hardware and forces dry-run, so a CLI
    /// session can never surprise a real fan controller.
    async fn open(cli: &Cli, use_mock: bool, profile: Option<Profile>) -> Result<Self> {
        let paths = match &cli.config_dir {
            Some(dir) => ConfigPaths::from_root(dir),
            None => ConfigPaths::discover()?,
        };
        paths.ensure()?;
        let store = SettingsStore::new(&paths);
        let mut settings = store.load()?;
        settings.log_level = cli.log_level.into();

        let mut options = AdapterOptions::from_settings(&settings);
        if use_mock {
            options.mock = true;
            options.protocol_device = true;
            options.mock_config = MockConfig {
                clock: ClockMode::Manual,
                ..options.mock_config
            };
            if let Some(profile) = profile {
                options.mock_config.gpu_load = profile.to_load_profile();
                options.mock_config.cpu_load = profile.to_load_profile();
            }
            settings.dry_run = true;
        }

        let adapters: Vec<Arc<dyn HardwareAdapter>> = build_adapters(&options);
        let runtime = Runtime::new(paths.clone(), settings, adapters)?;
        let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));
        Ok(Self {
            runtime,
            engine,
            paths,
        })
    }

    async fn start(&self) -> Result<()> {
        self.runtime.start().await?;
        self.engine.load_rules()?;
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.engine.stop().await;
        let release = self.runtime.shutdown().await?;
        report_control_release(&release);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // The CLI prints its own report, so logs stay out of the way by default. The
    // exception is the background service: nobody is watching its terminal, so it
    // must leave a file behind, next to the settings and the audit trail. Only the
    // first initialisation takes effect, which is why the decision is made here
    // rather than inside `service_run`.
    let log_paths = match &cli.command {
        Command::Service {
            action: ServiceAction::Run { .. },
        } => Some(match &cli.config_dir {
            Some(root) => ConfigPaths::from_root(root),
            None => ConfigPaths::discover()?,
        }),
        _ => None,
    };
    let _guard = ohm_core::logging::init(cli.log_level.into(), log_paths.as_ref())?;

    match &cli.command {
        Command::Doctor { mock, json } => doctor(&cli, *mock, *json).await,
        Command::Status { mock, json } => status(&cli, *mock, *json).await,
        Command::Watch {
            mock,
            interval,
            count,
        } => watch(&cli, *mock, *interval, *count).await,
        Command::Demo {
            profile,
            step_seconds,
            steps,
            ambient,
        } => demo(&cli, *profile, *step_seconds, *steps, *ambient).await,
        Command::Rules { action } => rules(&cli, action).await,
        Command::Audit { limit } => audit(&cli, *limit).await,
        Command::Handovers { retry } => handovers(&cli, *retry).await,
        Command::Paths => paths(&cli),
        Command::Protocol => protocol().await,
        Command::Service { action } => match action {
            ServiceAction::Run {
                mock,
                heartbeat_ms,
                max_ticks,
            } => service_run(&cli, *mock, *heartbeat_ms, *max_ticks).await,
            ServiceAction::Status => service_status(&cli),
        },
        Command::Report { out, mock } => report(&cli, out.as_deref(), *mock).await,
    }
}

fn paths(cli: &Cli) -> Result<()> {
    let paths = match &cli.config_dir {
        Some(dir) => ConfigPaths::from_root(dir),
        None => ConfigPaths::discover()?,
    };
    print_header("Configuration layout");
    println!("{}", paths.describe());
    println!();
    println!("Local first: nothing here is uploaded, and there is no account.");
    Ok(())
}

/// The doctor report in machine-readable form.
///
/// Field names are part of the interface a cross-check script reads, so they are
/// explicit rather than derived: `adapters[].info.id` is what `--json` consumers
/// match on, and `devices[].state.readings[].status` is how "missing, and why" is
/// expressed. A reading that is absent is absent, never 0.
#[derive(Debug, serde::Serialize)]
struct DoctorReport {
    version: String,
    os: String,
    arch: String,
    config_dir: String,
    simulated: bool,
    adapters: Vec<ohm_runtime::AdapterView>,
    devices: Vec<ohm_runtime::DeviceView>,
    capabilities: ohm_runtime::CapabilityIndex,
    notes: Vec<String>,
}

async fn doctor(cli: &Cli, use_mock: bool, json: bool) -> Result<()> {
    let session = Session::open(cli, use_mock, None).await?;
    session.start().await?;

    print_header("OpenHardwareOS doctor");
    println!("config:   {}", session.paths.root().display());
    println!("version:  {}", ohm_core::VERSION);
    println!(
        "platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if use_mock {
        println!("note:     simulated providers are enabled and writes are dry-run");
    }

    print_header("Providers");
    print_adapters(&session.runtime);

    print_header("Devices");
    print_devices(&session.runtime);

    let index = session.runtime.capability_index();
    print_header("Capabilities");
    println!(
        "  {} readable sensor(s), {} writable actuator(s)",
        index.sources.len(),
        index.targets.len()
    );
    if index.targets.is_empty() {
        println!();
        println!("  This machine exposes no controllable output.");
        println!("  That is expected: chassis fan control needs SuperIO access, which on");
        println!("  Windows comes from LibreHardwareMonitor (Options -> Remote Web Server");
        println!("  -> Run), and on other platforms generally does not exist at all.");
        println!("  Run `ohm-cli demo` to exercise the full cooling loop with simulated");
        println!("  hardware on any machine.");
    } else {
        for target in index.targets.iter().take(12) {
            println!("  • {}", target.qualified_id());
        }
    }

    // Capabilities that a reader of the MVP sensor list would expect, with an
    // explicit explanation when this machine cannot provide them. Never a zero,
    // never silence.
    let notes: Vec<String> = {
        let devices = session.runtime.devices();
        let mut notes = Vec::new();
        let has = |capability: &str| {
            devices
                .iter()
                .any(|view| view.device.capability_str(capability).is_some())
        };
        let readable = |capability: &str| {
            devices.iter().any(|view| {
                view.device.capability_str(capability).is_some()
                    && view
                        .state
                        .as_ref()
                        .and_then(|state| state.get(capability))
                        .is_some_and(|reading| reading.is_ok())
            })
        };

        if readable(caps::POWER_TOTAL) {
            notes.push(format!(
                "CPU package power is available from: {}",
                devices
                    .iter()
                    .filter(|view| view
                        .state
                        .as_ref()
                        .is_some_and(|state| state.number(caps::POWER_TOTAL).is_some()))
                    .map(|view| format!("{} ({})", view.device.name, view.adapter))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        } else {
            notes.push(
                "CPU package power: not available. There is no native collector in this build; \
                 on Windows it comes from LibreHardwareMonitor (Provider: lhm), which needs LHM \
                 running with its web server enabled. It is reported as missing rather than as \
                 0 W."
                    .to_string(),
            );
        }
        if !has(caps::TEMPERATURE_HOTSPOT) {
            notes.push(
                "GPU hotspot temperature: not available from any registered provider. NVML does \
                 not expose it; LibreHardwareMonitor reads it through NVAPI."
                    .to_string(),
            );
        }
        if !has(caps::FAN_SPEED_PERCENT) && !has(caps::PUMP_SPEED_PERCENT) {
            notes.push(
                "Fan control: no writable cooling channel. On Windows this needs \
                 LibreHardwareMonitor (SuperIO). Run `ohm-cli demo` for the simulated loop."
                    .to_string(),
            );
        }
        notes
    };

    if json {
        let report = DoctorReport {
            version: ohm_core::VERSION.to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            config_dir: session.paths.root().display().to_string(),
            simulated: use_mock,
            adapters: session.runtime.adapter_views(),
            devices: session.runtime.devices(),
            capabilities: session.runtime.capability_index(),
            notes: notes.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        session.stop().await?;
        return Ok(());
    }

    print_header("Capability notes");
    for note in notes {
        println!("  • {note}");
    }

    print_header("Rules");
    if session.engine.rules().is_empty() {
        println!("  no rules installed (try `ohm-cli rules suggest`)");
    } else {
        for outcome in session.engine.outcomes() {
            println!("  {}", outcome.summary());
        }
    }

    let stats = session.runtime.stats();
    print_header("Runtime");
    println!("  poll cycles:      {}", stats.poll_cycles);
    println!("  discovery cycles: {}", stats.discovery_cycles);
    println!(
        "  writes:           {} applied, {} rejected",
        stats.writes_applied, stats.writes_rejected
    );
    println!("  safety actions:   {}", stats.safety_interventions);

    session.stop().await?;
    Ok(())
}

/// One machine's report: our readings, and the platform's own answers beside them.
///
/// The shape is deliberately flat JSON that a person can read in a text editor and a
/// script can parse, because the file is meant to travel: from a Windows desktop or a
/// Linux box, to whoever has to decide whether a reading is real.
#[derive(Debug, serde::Serialize)]
struct FieldReport {
    generated_at_ms: i64,
    version: String,
    os: String,
    arch: String,
    /// `uname -r` where there is one, so "which kernel" is answered by the file.
    kernel: Option<String>,
    config_dir: String,
    simulated: bool,
    dry_run: bool,
    /// Providers with their status and the reason for it.
    providers: Vec<ohm_runtime::AdapterView>,
    /// Every device, its readings, and the reason for each missing one.
    devices: Vec<ohm_runtime::DeviceView>,
    capabilities: ohm_runtime::CapabilityIndex,
    /// What a reader of the MVP sensor list would expect and this machine cannot give.
    notes: Vec<String>,
    /// The rules installed here, as configured.
    rules: Vec<ReportRule>,
    /// The platform's own evidence, read directly rather than through this project.
    platform: PlatformEvidence,
}

#[derive(Debug, serde::Serialize)]
struct ReportRule {
    id: String,
    name: String,
    enabled: bool,
    source: String,
    target: String,
}

/// What the operating system itself says, gathered from files rather than from an API
/// this project wrote. A reader can compare these with the readings above.
/// What LibreHardwareMonitor reports, on a machine where it is running.
///
/// On Windows this *is* the platform's own answer: LHM owns the SuperIO driver and
/// reads the same registers we read through it, so its window is what a person
/// compares our readings against. Empty on a machine without LHM, with the reason.
#[derive(Debug, Default, serde::Serialize)]
struct LhmEvidence {
    url: String,
    sensors: Vec<ohm_adapters::prelude::LhmSensorEvidence>,
    /// Set when the list was cut short, so a reader knows it is not everything.
    truncated_at: Option<usize>,
    notes: Vec<String>,
}

#[derive(Debug, Default, serde::Serialize)]
struct PlatformEvidence {
    /// Where the hwmon files were read from — an override tree, or the real sysfs.
    hwmon_root: Option<String>,
    /// One entry per chip directory, with the raw contents of every fan, pwm and
    /// temperature file, exactly as the kernel presents them.
    hwmon: Vec<HwmonChipEvidence>,
    /// `MemTotal` and `MemAvailable` from `/proc/meminfo`, in the file's own units.
    meminfo: Vec<String>,
    /// `df -k` for the filesystems this report covers.
    df: Vec<String>,
    /// LibreHardwareMonitor's own sensor list, when it is running.
    lhm: LhmEvidence,
    /// Notes about what could not be read, so an empty section is explained.
    notes: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct HwmonChipEvidence {
    directory: String,
    name: Option<String>,
    /// `file name` → its raw contents, whitespace trimmed.
    files: std::collections::BTreeMap<String, String>,
    /// A file that could not be read, with the error.
    unreadable: std::collections::BTreeMap<String, String>,
}

/// Read the platform's own view of the machine.
///
/// Every failure here becomes a note rather than an error: a report from a machine
/// with no `hwmon` at all is still a useful report, and saying *why* a section is
/// empty is the whole point of collecting one.
fn platform_evidence(lhm: Option<ohm_adapters::prelude::LhmConfig>) -> PlatformEvidence {
    let mut evidence = PlatformEvidence::default();

    // LibreHardwareMonitor's own view, when it is there. Windows has no sysfs, so
    // without this the platform half of the report is empty on exactly the machine
    // the fan work happens on — and LHM's window is what a person compares against
    // there.
    match lhm {
        None => evidence.lhm.notes.push(
            "LibreHardwareMonitor is switched off in settings, so its own view of this \
             machine is not in this report"
                .to_string(),
        ),
        Some(config) => {
            evidence.lhm.url = config.base_url.clone();
            match ohm_adapters::prelude::LhmAdapter::new(config).platform_evidence() {
                Ok(sensors) => {
                    const LIMIT: usize = 400;
                    if sensors.len() > LIMIT {
                        evidence.lhm.truncated_at = Some(LIMIT);
                        evidence.lhm.sensors = sensors.into_iter().take(LIMIT).collect();
                    } else {
                        evidence.lhm.sensors = sensors;
                    }
                }
                Err(error) => evidence.lhm.notes.push(format!(
                    "LibreHardwareMonitor could not be read: {}",
                    error.detail(&evidence.lhm.url)
                )),
            }
        }
    }

    let root: Option<std::path::PathBuf> =
        match std::env::var_os(ohm_adapters::prelude::ENV_HWMON_ROOT) {
            Some(value) if value.is_empty() => None,
            Some(value) => Some(std::path::PathBuf::from(value)),
            None => {
                #[cfg(target_os = "linux")]
                {
                    Some(std::path::PathBuf::from("/sys/class/hwmon"))
                }
                #[cfg(not(target_os = "linux"))]
                {
                    None
                }
            }
        };

    match &root {
        None => evidence.notes.push(if cfg!(target_os = "linux") {
            format!(
                "hwmon: disabled by an empty {}. Nothing to compare fan readings with.",
                ohm_adapters::prelude::ENV_HWMON_ROOT
            )
        } else {
            "hwmon: this platform has no sysfs, so fan tachometers and PWM values have no file to compare with. On Windows the same numbers come from LibreHardwareMonitor, and its own sensor list is in this report."
                .to_string()
        }),
        Some(path) => {
            evidence.hwmon_root = Some(path.display().to_string());
            match std::fs::read_dir(path) {
                Err(error) => evidence.notes.push(format!("hwmon: cannot read {}: {error}", path.display())),
                Ok(entries) => {
                    let mut directories: Vec<std::path::PathBuf> = entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| path.is_dir())
                        .collect();
                    directories.sort();
                    for directory in directories {
                        let mut chip = HwmonChipEvidence {
                            directory: directory.display().to_string(),
                            name: None,
                            files: std::collections::BTreeMap::new(),
                            unreadable: std::collections::BTreeMap::new(),
                        };
                        if let Ok(name) = std::fs::read_to_string(directory.join("name")) {
                            chip.name = Some(name.trim().to_string());
                        }
                        if let Ok(files) = std::fs::read_dir(&directory) {
                            let mut names: Vec<(String, std::path::PathBuf)> = files
                                .flatten()
                                .map(|entry| {
                                    (
                                        entry.file_name().to_string_lossy().to_string(),
                                        entry.path(),
                                    )
                                })
                                .filter(|(name, _)| {
                                    name.starts_with("fan")
                                        || name.starts_with("pwm")
                                        || name.starts_with("temp")
                                })
                                .collect();
                            names.sort();
                            for (name, path) in names {
                                match std::fs::read_to_string(&path) {
                                    Ok(content) => {
                                        chip.files.insert(name, content.trim().to_string());
                                    }
                                    Err(error) => {
                                        chip.unreadable.insert(name, error.to_string());
                                    }
                                }
                            }
                        }
                        evidence.hwmon.push(chip);
                    }
                    if evidence.hwmon.is_empty() {
                        evidence.notes.push(format!(
                            "hwmon: {} exists but has no chip directories",
                            path.display()
                        ));
                    }
                }
            }
        }
    }

    match std::fs::read_to_string("/proc/meminfo") {
        Ok(text) => {
            evidence.meminfo = text
                .lines()
                .filter(|line| line.starts_with("MemTotal") || line.starts_with("MemAvailable"))
                .map(|line| line.trim().to_string())
                .collect();
            if evidence.meminfo.is_empty() {
                evidence.notes.push("/proc/meminfo has no MemTotal or MemAvailable line".into());
            }
        }
        Err(error) => evidence.notes.push(format!(
            "/proc/meminfo: not readable ({error}); on Windows and macOS the platform's own memory figure comes from its own tools"
        )),
    }

    // `df -k` for the filesystems this machine reports, so the free-space reading has
    // a second opinion beside it. Coarse on purpose: the point is the order of
    // magnitude and the mount point, not a byte-for-byte match against a moving number.
    match std::process::Command::new("df").arg("-k").output() {
        Ok(output) if output.status.success() => {
            evidence.df = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim_end().to_string())
                .collect();
        }
        Ok(output) => evidence
            .notes
            .push(format!("df -k exited with {}", output.status)),
        Err(error) => evidence
            .notes
            .push(format!("df is not available here: {error}")),
    }

    evidence
}

/// `uname -r`, where there is a `uname`.
fn kernel_version() -> Option<String> {
    let output = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

async fn report(cli: &Cli, out: Option<&str>, use_mock: bool) -> Result<()> {
    let session = Session::open(cli, use_mock, None).await?;
    session.start().await?;

    let lhm_config = ohm_adapters::prelude::LhmConfig::from_json(
        session
            .runtime
            .settings()
            .adapter_config(ohm_adapters::prelude::LHM_ADAPTER_ID),
    );
    let platform = platform_evidence(
        session
            .runtime
            .settings()
            .adapter_enabled(ohm_adapters::prelude::LHM_ADAPTER_ID)
            .then_some(lhm_config),
    );

    let settings = session.runtime.settings();
    let rules = session
        .engine
        .rules()
        .into_iter()
        .map(|rule| ReportRule {
            id: rule.id.to_string(),
            name: rule.name.clone(),
            enabled: rule.enabled,
            source: rule.source.label(),
            target: rule.target.qualified_id(),
        })
        .collect();

    let report = FieldReport {
        generated_at_ms: ohm_core::now_ms(),
        version: ohm_core::VERSION.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        kernel: kernel_version(),
        config_dir: session.paths.root().display().to_string(),
        simulated: use_mock,
        dry_run: settings.dry_run,
        providers: session.runtime.adapter_views(),
        devices: session.runtime.devices(),
        capabilities: session.runtime.capability_index(),
        notes: capability_notes(&session),
        rules,
        platform,
    };

    let path = match out {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            std::fs::create_dir_all(session.paths.reports_dir())?;
            session
                .paths
                .reports_dir()
                .join(format!("report-{}.json", ohm_core::now_ms()))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;

    print_header("OpenHardwareOS report");
    println!("written to: {}", path.display());
    println!(
        "version:    {} on {} {}",
        report.version,
        report.os,
        report.kernel.as_deref().unwrap_or("(no uname)")
    );
    println!(
        "providers:  {}",
        report
            .providers
            .iter()
            .map(|view| format!("{}={}", view.id(), view.status.state.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("devices:    {}", report.devices.len());
    println!(
        "platform:   {} hwmon chip(s), {} meminfo line(s), {} df line(s), {} LHM sensor(s)",
        report.platform.hwmon.len(),
        report.platform.meminfo.len(),
        report.platform.df.len(),
        report.platform.lhm.sensors.len()
    );
    for note in &report.platform.lhm.notes {
        println!("note:       {note}");
    }
    for note in &report.platform.notes {
        println!("note:       {note}");
    }
    println!();
    println!("Send this file with `ohm-cli doctor` output and, on Linux, the output of");
    println!("`scripts/verify-linux-readings.sh`. See docs/field-checklist.md.");

    session.stop().await?;
    Ok(())
}

/// The capability notes `doctor` prints, reused so a report carries the same
/// explanation of what this machine cannot give.
fn capability_notes(session: &Session) -> Vec<String> {
    let devices = session.runtime.devices();
    let mut notes = Vec::new();
    let has = |capability: &str| {
        devices
            .iter()
            .any(|view| view.device.capability_str(capability).is_some())
    };
    let readable = |capability: &str| {
        devices.iter().any(|view| {
            view.device.capability_str(capability).is_some()
                && view
                    .state
                    .as_ref()
                    .and_then(|state| state.get(capability))
                    .is_some_and(|reading| reading.is_ok())
        })
    };
    if readable(ohm_core::ids::capability::POWER_TOTAL) {
        notes.push("CPU package power is available from this machine".to_string());
    } else {
        notes.push(
            "CPU package power: not available. There is no native collector in this build;              on Windows it comes from LibreHardwareMonitor (Provider: lhm), which needs LHM              running with its web server enabled."
                .to_string(),
        );
    }
    if !has(ohm_core::ids::capability::TEMPERATURE_HOTSPOT) {
        notes.push(
            "GPU hotspot temperature: not available from any registered provider.".to_string(),
        );
    }
    if !has(ohm_core::ids::capability::FAN_SPEED_PERCENT)
        && !has(ohm_core::ids::capability::PUMP_SPEED_PERCENT)
    {
        notes.push(
            "Fan control: no writable cooling channel on this machine. On Windows this              needs LibreHardwareMonitor (SuperIO); on Linux this build does not write PWM              at all."
                .to_string(),
        );
    }
    notes
}

/// A compact picture of every rule, for the state file.
///
/// Deliberately short: this is read by a person asking "is this machine being cooled
/// the way I asked", and by nothing that parses it as an interface.
fn rule_outcomes(session: &Session) -> Vec<ServiceOutcome> {
    session
        .engine
        .outcomes()
        .into_iter()
        .take(32)
        .map(|outcome| ServiceOutcome {
            id: outcome.rule_id.to_string(),
            status: format!("{:?}", outcome.status).to_lowercase(),
            applied: outcome.applied_output,
            message: outcome.message.clone(),
        })
        .collect()
}

/// Wait for the signal a service is stopped with.
///
/// `Ctrl-C` everywhere, plus `SIGTERM` on Unix, because that is what a service
/// manager sends. Both mean the same thing here: stop taking new decisions, make
/// the machine safe, and say what happened.
async fn wait_for_stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, "no SIGTERM handler; waiting for Ctrl-C only");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The background service: the runtime and the rule loop, with no window.
async fn service_run(cli: &Cli, use_mock: bool, heartbeat_ms: u64, max_ticks: u64) -> Result<()> {
    let session = Session::open(cli, use_mock, None).await?;
    let settings = session.runtime.settings();

    // The stop signal is handled *before* the state file is claimed, not after.
    // A service manager may send SIGTERM the moment the process looks alive, and a
    // service that has claimed the channels but cannot yet hear the signal is a
    // process that dies by default termination — leaving the state file behind and
    // the reason unrecorded. Being ready to stop comes first.
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal_waiter = {
        let stop = std::sync::Arc::clone(&stop);
        tokio::spawn(async move {
            wait_for_stop_signal().await;
            // `notify_one` leaves a permit when nobody is waiting yet, so a signal
            // that arrives between heartbeats is not lost.
            stop.notify_one();
        })
    };

    // Claim the channels before touching them: a second service must be refused
    // rather than allowed to alternate values onto the same fan.
    let state_path = session.paths.service_state_file();
    let state = ServiceState {
        pid: std::process::id(),
        version: ohm_core::VERSION.to_string(),
        started_at_ms: ohm_core::now_ms(),
        heartbeat_at_ms: ohm_core::now_ms(),
        heartbeat_interval_ms: heartbeat_ms.max(50),
        ticks: 0,
        rules: 0,
        outcomes: Vec::new(),
        simulated: use_mock,
        dry_run: settings.dry_run,
    };
    let mut guard = match ServiceGuard::acquire(&state_path, state) {
        Ok(guard) => guard,
        Err(ServiceError::AlreadyRunning { state, age_ms }) => {
            signal_waiter.abort();
            eprintln!(
                "error: a service is already running here (pid {}, started {}, last heartbeat {} ms ago).",
                state.pid,
                ohm_runtime::release::format_ms(state.started_at_ms),
                age_ms
            );
            eprintln!("       Two services would fight over the same channels. Stop it first,");
            eprintln!(
                "       or delete {} if it is not coming back.",
                state_path.display()
            );
            std::process::exit(3);
        }
        Err(error) => return Err(error.into()),
    };

    session.runtime.start().await?;
    session.engine.start().await?;

    let rules = session.engine.stats();
    guard.heartbeat(rules.ticks, rules.rules, rule_outcomes(&session))?;

    print_header("OpenHardwareOS service");
    println!("state file: {}", state_path.display());
    println!("log file:   {}", session.paths.logs_dir().display());
    println!("pid:        {}", std::process::id());
    println!("version:    {}", ohm_core::VERSION);
    println!(
        "providers:  {}",
        if use_mock {
            "simulated hardware, writes dry-run"
        } else {
            "this machine"
        }
    );
    println!(
        "rules:      {} loaded, {} enabled",
        rules.rules, rules.enabled_rules
    );
    println!(
        "automation: {}",
        if settings.automation_enabled {
            "enabled"
        } else {
            "disabled in settings — this service will not write to hardware"
        }
    );
    println!();
    println!("Running. Stop with Ctrl-C, or SIGTERM from a service manager.");
    println!("`ohm-cli service status` reports from the state file while this runs.");

    // Written to a file or a pipe, stdout is block-buffered: without this the
    // banner can still be in memory when the process is asked to stop.
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let mut heartbeat = tokio::time::interval(Duration::from_millis(heartbeat_ms.max(50)));
    let reason = loop {
        tokio::select! {
            _ = stop.notified() => break "stop requested",
            _ = heartbeat.tick() => {
                let stats = session.engine.stats();
                guard.heartbeat(stats.ticks, stats.rules, rule_outcomes(&session))?;
                if max_ticks > 0 && stats.ticks >= max_ticks {
                    break "reached --max-ticks";
                }
            }
        }
    };
    signal_waiter.abort();

    let stats = session.engine.stats();
    println!();
    println!("{reason} after {} automation cycle(s)", stats.ticks);
    // Stopping is where control goes back: `Session::stop` stops the rule loop,
    // shuts the runtime down and reports what the release actually achieved —
    // confirmed, unconfirmed, refused, failed, simulated — rather than claiming
    // success for a write nobody saw land.
    session.stop().await?;
    // `guard` is dropped here, which removes the state file: a clean stop leaves
    // nothing that could make the next start think a service is running.
    drop(guard);
    println!("state file removed; nothing is driving the channels now");
    Ok(())
}

/// Report what the state file says, without touching the runtime.
fn service_status(cli: &Cli) -> Result<()> {
    let paths = match &cli.config_dir {
        Some(root) => ConfigPaths::from_root(root),
        None => ConfigPaths::discover()?,
    };
    let path = paths.service_state_file();
    print_header("OpenHardwareOS service");
    println!("state file: {}", path.display());
    match ohm_runtime::service::stale_or_running(&path) {
        None => {
            println!("running:    no");
            println!(
                "            No service has written a state file here. Start one with `ohm-cli service run`."
            );
            std::process::exit(1);
        }
        Some((state, true)) => {
            let age = state.age_ms(ohm_core::now_ms());
            println!("running:    yes");
            println!("pid:        {}", state.pid);
            println!("version:    {}", state.version);
            println!(
                "started:    {} ({} s ago)",
                ohm_runtime::release::format_ms(state.started_at_ms),
                (ohm_core::now_ms() - state.started_at_ms) / 1000
            );
            println!(
                "heartbeat:  {age} ms ago (every {} ms)",
                state.heartbeat_interval_ms
            );
            println!("cycles:     {}", state.ticks);
            println!("rules:      {}", state.rules);
            println!(
                "providers:  {}",
                if state.simulated {
                    "simulated hardware"
                } else {
                    "this machine"
                }
            );
            if state.dry_run {
                println!("writes:     dry-run — nothing reaches hardware");
            }
            if state.outcomes.is_empty() {
                println!("rules:      none installed");
            } else {
                println!();
                println!("rules:");
                for outcome in &state.outcomes {
                    match outcome.applied {
                        Some(applied) => println!(
                            "  {:<24} {:<12} at {applied:.0}  {}",
                            outcome.id, outcome.status, outcome.message
                        ),
                        None => println!(
                            "  {:<24} {:<12}         {}",
                            outcome.id, outcome.status, outcome.message
                        ),
                    }
                }
            }
            Ok(())
        }
        Some((state, false)) => {
            let age = state.age_ms(ohm_core::now_ms());
            println!("running:    no");
            println!(
                "            A state file from pid {} is here, last heartbeat {} ms ago ({:.1} min).",
                state.pid,
                age,
                age as f64 / 60_000.0
            );
            println!("            That process is not keeping it up to date, so it is treated as");
            println!("            stopped; the next `service run` takes the file over.");
            std::process::exit(1);
        }
    }
}

async fn status(cli: &Cli, use_mock: bool, json: bool) -> Result<()> {
    let session = Session::open(cli, use_mock, None).await?;
    session.start().await?;
    if json {
        // The same snapshot the desktop renders from, so the two cannot disagree
        // about what the runtime reported.
        println!(
            "{}",
            serde_json::to_string_pretty(&session.runtime.snapshot())?
        );
        session.stop().await?;
        return Ok(());
    }
    print_header("Status");
    print_devices(&session.runtime);
    session.stop().await?;
    Ok(())
}

async fn watch(cli: &Cli, use_mock: bool, interval: f64, count: u32) -> Result<()> {
    let session = Session::open(cli, use_mock, None).await?;
    session.start().await?;
    let interval = Duration::from_secs_f64(interval.clamp(0.1, 60.0));

    let mut ticker = tokio::time::interval(interval);
    let mut seen = 0u32;
    let mut events = session.runtime.subscribe();

    loop {
        ticker.tick().await;
        // Drain the event bus so its channel never fills up.
        while events.try_recv().is_ok() {}

        println!();
        println!(
            "  {}   poll #{} · {} writes · {} safety actions",
            ohm_core::now_rfc3339(),
            session.runtime.stats().poll_cycles,
            session.runtime.stats().writes_applied,
            session.runtime.stats().safety_interventions
        );
        print_devices(&session.runtime);

        seen += 1;
        if count > 0 && seen >= count {
            break;
        }
    }

    session.stop().await?;
    Ok(())
}

/// Acceptance scenario C, headless and time-compressed.
async fn demo(
    cli: &Cli,
    profile: Profile,
    step_seconds: f64,
    steps: u32,
    ambient: f64,
) -> Result<()> {
    print_header("OpenHardwareOS demo — simulated cooling loop");
    println!("  profile         : {profile:?}");
    println!("  ambient         : {ambient:.0} °C");
    println!("  simulated step  : {step_seconds:.1} s × {steps} steps");

    let paths = match &cli.config_dir {
        Some(dir) => ConfigPaths::from_root(dir),
        None => ConfigPaths::discover()?,
    };
    paths.ensure()?;

    // Simulated hardware only: no real fan is ever touched by this command.
    let mut options = AdapterOptions::simulated_only();
    let mut mock_config = MockConfig::deterministic();
    mock_config.ambient_c = ambient;
    mock_config.gpu_load = profile.to_load_profile();
    mock_config.cpu_load = profile.to_load_profile();
    // Start hot so the loop has something to do immediately.
    mock_config.gpu_start_c = 78.0;
    options.mock_config = mock_config;

    let adapters = build_adapters(&options);
    // Keep our own handle on the simulator: the runtime owns the adapters, but
    // this command needs to drive the simulated clock directly.
    let mock_position = adapters
        .iter()
        .position(|adapter| adapter.info().id.as_str() == ohm_adapter_mock::ADAPTER_ID)
        .context("the simulated provider must be registered")?;
    let mock_handle = Arc::clone(&adapters[mock_position]);

    let mut settings = SettingsStore::new(&paths).load().unwrap_or_default();
    settings.polling_interval_ms = 1_000;
    settings.automation_enabled = true;
    // Writes go to the simulation, which is the entire point of this command.
    settings.dry_run = false;

    let runtime = Runtime::new(paths.clone(), settings, adapters)?;
    let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));
    let mock: &MockAdapter = mock_handle
        .as_any()
        .downcast_ref::<MockAdapter>()
        .context("the simulated provider has an unexpected type")?;

    runtime.start().await?;
    engine.load_rules()?;

    if engine.rule("demo-gpu-cooling").is_none() {
        let rule = Rule::new(
            "demo-gpu-cooling",
            "Demo GPU Cooling",
            ohm_automation::Source::sensor("gpu.mock.0", caps::TEMPERATURE_CORE),
            ohm_automation::Target::new("fan.mock.0", caps::FAN_SPEED_PERCENT),
            ohm_automation::gpu_cooling_curve(),
        )?
        .with_description("Installed by `ohm-cli demo`; safe to delete.")
        .with_hysteresis(2.0)
        .with_deadband(1.0);
        engine.save_rule(rule)?;
        println!();
        println!("  installed rule  : demo-gpu-cooling (GPU temp -> chassis fan)");
    }

    println!();
    println!(
        "  {:>7}  {:>7}  {:>7}  {:>9}  {:>8}  {:>10}",
        "sim s", "cpu °C", "gpu °C", "fan duty", "fan RPM", "rule"
    );

    let mut hottest = 0.0f64;
    let mut start_temp = 0.0f64;
    let mut ever_wrote = false;

    for step in 0..steps {
        // Advance the simulation, publish it through the runtime, then let the
        // engine react — exactly the order the desktop app uses.
        mock.tick((step_seconds * 1000.0) as u64);
        runtime.poll_once().await?;
        engine.tick_force().await;

        let status = mock.status();
        if step == 0 {
            start_temp = status.gpu_temp_c;
        }
        hottest = hottest.max(status.gpu_temp_c);
        let outcome = engine.outcome("demo-gpu-cooling");
        if outcome.as_ref().is_some_and(|o| o.writes > 0) {
            ever_wrote = true;
        }
        let rule_note = outcome
            .as_ref()
            .map(|o| format!("{:?}", o.status).to_lowercase())
            .unwrap_or_else(|| "none".into());

        if step % 5 == 0 || step + 1 == steps {
            println!(
                "  {:>7.0}  {:>7.1}  {:>7.1}  {:>8.0}%  {:>8.0}  {:>10}",
                status.sim_ms as f64 / 1000.0,
                status.cpu_temp_c,
                status.gpu_temp_c,
                status.fan_duties.first().copied().unwrap_or(0.0),
                status.fan_rpms.first().copied().unwrap_or(0.0),
                rule_note
            );
        }
    }

    let final_status = mock.status();
    let outcome = engine.outcome("demo-gpu-cooling");

    print_header("Result");
    println!("  start GPU temperature : {start_temp:.1} °C");
    println!("  hottest GPU           : {hottest:.1} °C");
    println!(
        "  final GPU temperature : {:.1} °C",
        final_status.gpu_temp_c
    );
    println!(
        "  final fan duty        : {:.0} % ({:.0} RPM)",
        final_status.fan_duties.first().copied().unwrap_or(0.0),
        final_status.fan_rpms.first().copied().unwrap_or(0.0)
    );
    if let Some(outcome) = outcome {
        println!(
            "  rule                  : {} — {} evaluations, {} writes, {} skipped, {} fallbacks",
            outcome.message,
            outcome.evaluations,
            outcome.writes,
            outcome.skipped,
            outcome.fallbacks
        );
    }
    let index = runtime.capability_index();
    println!(
        "  closed loop           : {} sensor(s) drove {} actuator(s)",
        index.sources.len(),
        index.targets.len()
    );
    if let Some(entry) = runtime.audit().tail(1).unwrap_or_default().last() {
        println!("  last audit entry      : {entry}");
    }

    engine.stop().await;
    let release = runtime.shutdown().await?;
    report_control_release(&release);

    if !ever_wrote {
        anyhow::bail!("the demo never wrote to the simulated fan; the loop is broken");
    }
    Ok(())
}

/// Show what happened to channels that rules stopped driving.
///
/// An unfinished handover is the one piece of automation state that must never be
/// invisible: the channel is still sitting at whatever the abandoned curve last asked
/// for, and the rule that abandoned it may itself be gone. The record names that rule
/// as data, and `--retry` is how the user asks for another attempt once whatever was
/// broken has been fixed.
async fn handovers(cli: &Cli, retry: bool) -> Result<()> {
    let session = Session::open(cli, false, None).await?;
    session.start().await?;
    // Read the machine once and check anything recovered from a previous session
    // against it, so this report is about the hardware rather than about the file.
    // Verification resolves channels and marks what it finds; it writes nothing.
    let _ = session.runtime.poll_once().await;
    session.engine.verify_recovered_state();

    print_header("Control handovers");
    if retry {
        let rearmed = session.engine.retry_failed_handovers();
        println!("  re-armed {rearmed} handover(s) that had run out of attempts");
        if rearmed > 0 {
            // One tick performs the attempt, so the report below is not stale.
            session.engine.tick_force().await;
        }
    }

    let records = session.engine.handovers();
    if records.is_empty() {
        println!("  nothing outstanding: every channel a rule left behind was handed over");
        return Ok(());
    }
    for report in &records {
        for line in describe_handover(report) {
            println!("{line}");
        }
    }
    if let Some(error) = session.engine.persistence_error() {
        println!();
        println!("  NOT RECORDED: {error}");
        println!(
            "  Until this is fixed, a channel that is owed the fail-safe duty may not be \
             recovered after a restart."
        );
    }
    // The distinction matters: an unresolved handover means a channel nobody protects.
    let owed = session.engine.unfinished_handovers();
    if owed > 0 {
        println!();
        println!(
            "  {owed} channel(s) are still not protected by any rule or by the fail-safe duty"
        );
    }
    Ok(())
}

/// The lines a user sees for one handover.
///
/// Split out of the command so the text can be asserted on: an unfinished handover is
/// the state that must never be invisible, and "the CLI would print something" is not
/// evidence that it prints the reason, the channel and the way out.
fn describe_handover(report: &ohm_automation::HandoverReport) -> Vec<String> {
    let mut lines = vec![
        format!("  {}", report.summary()),
        format!("      reason : {}", report.reason),
    ];
    if let Some(error) = &report.first_error {
        lines.push(format!("      error  : {error}"));
    }
    match report.state {
        ohm_automation::HandoverState::Pending => lines.push(format!(
            "      state  : attempted {} time(s), will be retried",
            report.attempts
        )),
        ohm_automation::HandoverState::Failed => lines.push(format!(
            "      action : retrying stopped after {} attempt(s); fix the channel and run \
             `ohm-cli handovers --retry`",
            report.attempts
        )),
        ohm_automation::HandoverState::NeedsVerification => lines.push(
            "      state  : recovered from a previous session and not checked yet — the device \
             is verified before the fail-safe duty is applied"
                .to_string(),
        ),
        ohm_automation::HandoverState::AwaitingOwner => lines.push(format!(
            "      state  : claimed by `{}`, which has not driven it ({} tick(s) waiting) — the \
             channel is not protected while this lasts",
            report
                .claimant
                .as_ref()
                .map(ohm_core::RuleId::as_str)
                .unwrap_or("another rule"),
            report.claimed_ticks
        )),
        ohm_automation::HandoverState::Confirmed => {}
        ohm_automation::HandoverState::Superseded => {}
    }
    lines
}

async fn rules(cli: &Cli, action: &RulesAction) -> Result<()> {
    let use_mock = matches!(action, RulesAction::Suggest { mock: true });
    let session = Session::open(cli, use_mock, None).await?;
    session.start().await?;

    match action {
        RulesAction::List => {
            print_header("Automation rules");
            if session.engine.rules().is_empty() {
                println!("  none installed yet");
            }
            for outcome in session.engine.outcomes() {
                println!(
                    "  {:<22} {:<9} {} -> {}",
                    outcome.rule_id.as_str(),
                    format!("{:?}", outcome.status).to_lowercase(),
                    outcome.source,
                    outcome.target
                );
                println!("      {}", outcome.message);
            }
        }
        RulesAction::Suggest { .. } => {
            let existing = session.engine.rules();
            let suggestions = suggest_for(&session.runtime);
            let new_rules = merge_suggestions(&existing, suggestions);
            print_header("Suggested rules");
            if new_rules.is_empty() {
                println!("  nothing to suggest: hardware is missing or the rules already exist");
            }
            for rule in new_rules {
                println!("  • {} ({})", rule.name, rule.id);
                println!("      {}", rule.summary());
                session.engine.save_rule(rule)?;
            }
            println!();
            println!(
                "  rules are stored in {}",
                session.engine.store().directory().display()
            );
        }
        RulesAction::Delete { id } => {
            let removed = session.engine.delete_rule(id)?;
            println!(
                "  {}",
                if removed {
                    format!("deleted `{id}`")
                } else {
                    format!("no such rule: `{id}`")
                }
            );
        }
        RulesAction::SetEnabled { id, enabled } => {
            let rule = session.engine.set_rule_enabled(id, *enabled)?;
            println!(
                "  rule `{}` is now {}",
                rule.id,
                if rule.enabled { "enabled" } else { "disabled" }
            );
        }
        RulesAction::Check { path } => {
            let raw = std::fs::read_to_string(path)?;
            let rule: Rule = serde_yaml_ng::from_str(&raw)?;
            let check = session.engine.check_rule(&rule);
            println!("  rule: {} ({})", rule.name, rule.id);
            if check.errors.is_empty() {
                println!("  valid against the attached hardware");
            }
            for error in &check.errors {
                println!("  error:   {error}");
            }
            for warning in &check.warnings {
                println!("  warning: {warning}");
            }
            if !check.is_ok() {
                anyhow::bail!("the rule has {} error(s)", check.errors.len());
            }
        }
    }

    session.stop().await?;
    Ok(())
}

async fn audit(cli: &Cli, limit: usize) -> Result<()> {
    let paths = match &cli.config_dir {
        Some(dir) => ConfigPaths::from_root(dir),
        None => ConfigPaths::discover()?,
    };
    let log = ohm_runtime::AuditLog::new(paths.audit_log());
    let entries = log.tail(limit)?;
    print_header("Audit log");
    println!("  file: {}", paths.audit_log().display());
    if entries.is_empty() {
        println!("  nothing recorded yet");
        return Ok(());
    }
    for entry in entries {
        let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let at = entry.get("at").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "write" => {
                let report = &entry["report"];
                println!(
                    "  {at}  write      {} {} = {} [{}]",
                    report["device_id"].as_str().unwrap_or("?"),
                    report["capability"].as_str().unwrap_or("?"),
                    report
                        .get("applied")
                        .filter(|v| !v.is_null())
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| report["requested"].to_string()),
                    report["status"].as_str().unwrap_or("?")
                );
            }
            "lifecycle" => println!(
                "  {at}  lifecycle  {} — {}",
                entry.get("action").and_then(|v| v.as_str()).unwrap_or(""),
                entry.get("detail").and_then(|v| v.as_str()).unwrap_or("")
            ),
            other => println!("  {at}  {other}"),
        }
    }
    Ok(())
}

/// Show an Open Device Protocol exchange end to end.
async fn protocol() -> Result<()> {
    use ohm_protocol::mock::MockOpenFan;
    use ohm_protocol::{DeviceTransport, LoopbackTransport, Request, Response, handshake};

    print_header("Open Device Protocol exchange (simulated OpenFan)");
    let mut transport = LoopbackTransport::new(MockOpenFan::new(4));

    println!("  host   -> GET_DEVICE_INFO");
    let descriptor = handshake(&mut transport)?;
    println!("  device -> {}", descriptor.summary());
    println!(
        "            device id would be `{}`",
        descriptor.device_id(0)
    );

    println!();
    println!("  host   -> GET_CAPABILITIES");
    if let Response::Capabilities { capabilities } = transport.call(Request::GetCapabilities)? {
        for capability in capabilities {
            println!(
                "            {:<24} {:<9} {:<9} {}{}",
                capability.id,
                capability.kind.as_str(),
                capability.unit.as_str(),
                if capability.writable {
                    "writable"
                } else {
                    "read-only"
                },
                capability
                    .min
                    .zip(capability.max)
                    .map(|(min, max)| format!("  ({min:.0}-{max:.0})"))
                    .unwrap_or_default()
            );
        }
    }

    println!();
    println!("  host   -> GET_STATE");
    if let Response::State { readings } = transport.call(Request::GetState { capability: None })? {
        for reading in readings {
            let value = match reading.value() {
                Some(value) => value.to_string(),
                None => format!(
                    "[{}]",
                    reading
                        .reason()
                        .unwrap_or(ohm_device_model::UnavailableReason::Unknown)
                        .as_str()
                ),
            };
            println!("            {:<24} {value}", reading.capability.as_str());
        }
    }

    println!();
    println!("  host   -> SET_STATE fan.speed_percent = 85");
    let response = transport.call(Request::SetState {
        capability: caps::FAN_SPEED_PERCENT.into(),
        value: Value::Number(85.0),
    })?;
    println!("  device -> {response:?}");

    println!();
    println!("  device-side safety: after 5 s without host contact the device engages");
    println!("  its own fallback curve instead of holding whatever we last wrote.");
    println!();
    println!("  The real OpenHub (v0.4) and OpenFan (v0.5) will speak exactly this");
    println!("  protocol over USB HID or USB CDC. Only the transport changes: the");
    println!("  runtime, the automation engine and the UI stay as they are.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
        assert_eq!(truncate("温度传感器名称很长", 4), "温度传…");
    }

    /// The failed-handover text has to carry the channel, the rule that left it, the
    /// original error and the way out — that is the whole point of the record.
    #[test]
    fn an_unfinished_handover_is_described_with_a_way_out() {
        let report = ohm_automation::HandoverReport {
            device: ohm_core::DeviceId::new("fan.mock.0").unwrap(),
            capability: ohm_core::CapabilityId::new("fan.speed_percent").unwrap(),
            from_rule: ohm_core::RuleId::new("gone-rule").unwrap(),
            reason: "rule `gone-rule` was deleted; fan.mock.0/fan.speed_percent is no longer \
                     driven by it"
                .into(),
            state: ohm_automation::HandoverState::Failed,
            attempts: 5,
            first_error: Some("the device refused the fail-safe duty".into()),
            last_error: Some("the device refused the fail-safe duty".into()),
            queued_at_ms: 1,
            last_attempt_ms: 2,
            confirmed_value: None,
            superseded_by: None,
            claimant: None,
            claimed_ticks: 0,
        };
        let text = describe_handover(&report).join("\n");
        assert!(text.contains("fan.mock.0/fan.speed_percent"), "{text}");
        assert!(
            text.contains("gone-rule"),
            "the rule name survives its deletion: {text}"
        );
        assert!(text.contains("failed"), "{text}");
        assert!(
            text.contains("the device refused the fail-safe duty"),
            "{text}"
        );
        assert!(
            text.contains("--retry"),
            "the way out must be named: {text}"
        );
        assert!(
            !text.contains("applied"),
            "an unfinished handover is not a success: {text}"
        );

        let pending = ohm_automation::HandoverReport {
            state: ohm_automation::HandoverState::Pending,
            attempts: 2,
            ..report.clone()
        };
        let text = describe_handover(&pending).join("\n");
        assert!(text.contains("retried"), "{text}");
        assert!(
            !text.contains("retrying stopped"),
            "a pending handover is still being worked on: {text}"
        );
    }

    /// A handover waiting for a claimant to take control is not settled, and the text
    /// must say who it is waiting for.
    #[test]
    fn a_handover_waiting_for_its_claimant_says_so() {
        let report = ohm_automation::HandoverReport {
            device: ohm_core::DeviceId::new("fan.mock.0").unwrap(),
            capability: ohm_core::CapabilityId::new("fan.speed_percent").unwrap(),
            from_rule: ohm_core::RuleId::new("old-rule").unwrap(),
            reason: "rule `old-rule` was retargeted".into(),
            state: ohm_automation::HandoverState::AwaitingOwner,
            attempts: 1,
            first_error: None,
            last_error: None,
            queued_at_ms: 1,
            last_attempt_ms: 1,
            confirmed_value: None,
            superseded_by: None,
            claimant: Some(ohm_core::RuleId::new("r2").unwrap()),
            claimed_ticks: 7,
        };
        let text = describe_handover(&report).join("\n");
        assert!(text.contains("awaiting_owner"), "{text}");
        assert!(text.contains("r2"), "the claimant must be named: {text}");
        assert!(text.contains('7'), "and how long it has waited: {text}");
        assert!(
            text.contains("not protected"),
            "and that the channel is unprotected meanwhile: {text}"
        );
    }

    #[test]
    fn missing_readings_render_as_reasons_not_zero() {
        let device = ohm_core::DeviceId::new("fan.mock.0").unwrap();
        let state =
            DeviceState::new(device, 0).with_reading(ohm_device_model::Reading::unavailable(
                caps::FAN_RPM,
                ohm_device_model::UnavailableReason::PermissionDenied,
                Some("elevate".into()),
            ));
        let rendered = render(&Some(state), caps::FAN_RPM, " RPM");
        assert_eq!(rendered, "[permission_denied]");
        assert_eq!(render(&None, caps::FAN_RPM, " RPM"), "—");
    }
}
