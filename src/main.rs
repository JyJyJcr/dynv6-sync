mod resolve;
use std::{collections::HashMap, fmt, path::PathBuf};

use nix::{
    fcntl::{Flock, OFlag, open},
    sys::stat::Mode,
};

use clap::{Parser, ValueEnum};
use dynv6_rs::{AccessToken, Client, Record, RecordNode, RecordValue, ZoneID, ZoneValue};
use futures::future::join_all;
use itertools::Itertools;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::resolve::{PreRecord, Resolvable};

#[derive(Parser, Debug)]
struct Args {
    #[clap(help = "Path to the configuration file")]
    conf: PathBuf,
    #[clap(help = "Path to the variable file")]
    vars: PathBuf,
    #[clap(flatten)]
    log_params: LogArg,
    //#[clap(short = 'p', num_args = 2)]
    //variables: Vec<(String, String)>,
    #[clap(short = 'u', long = "update", num_args = 2, help = "Update variable")]
    keyvalues: Vec<String>,
    #[clap(
        long = "nosync",
        help = "Do not sync the records, just update variables"
    )]
    no_sync: bool,
}

#[derive(Parser, Debug)]
pub struct LogArg {
    #[clap(short = 'L', long = "log-out", default_value = "stdout")]
    log_out: LogOut,
    #[clap(short = 'l', long = "log-level", default_value = "info")]
    log_level: String,
}

#[derive(ValueEnum, Clone, Debug)]
enum LogOut {
    Stdout,
    #[cfg(target_os = "linux")]
    Journald,
}

#[derive(Deserialize, Debug)]
struct Config {
    token_path: PathBuf,
    domain: String,
    retry: usize,
    records: Vec<PreRecord>,
}

fn main() -> anyhow::Result<()> {
    let exe = std::env::args().nth(0).unwrap();
    let fd = open(exe.as_str(), OFlag::empty(), Mode::empty())?;
    let lock = match Flock::lock(fd, nix::fcntl::FlockArg::LockExclusive) {
        Ok(l) => l,
        Err(_) => return Err(anyhow::anyhow!("Failed to lock the file: {}", exe)),
    };
    //println!("LOCK");
    inner_main()?;
    drop(lock);
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn inner_main() -> anyhow::Result<()> {
    //sleep(std::time::Duration::from_secs(2)).await; // for testing
    //return Ok(());
    let args = Args::parse();
    // start tracing
    let leveled_subsc = tracing_subscriber::registry().with(
        Into::<EnvFilter>::into(&args.log_params.log_level),
        //     format!(
        //     "{}=trace,tower_http=trace,axum::rejection=trace",
        //     env!("CARGO_CRATE_NAME")
        // )
        //"trace",
    );
    match args.log_params.log_out {
        LogOut::Stdout => {
            leveled_subsc.with(tracing_subscriber::fmt::layer()).init();
        }
        #[cfg(target_os = "linux")]
        LogOut::Journald => {
            leveled_subsc.with(tracing_journald::layer()?).init();
        }
    }
    tracing::info!(
        "tracing initialized with level: {}",
        args.log_params.log_level
    );

    tracing::debug!("args: {:?}", args);

    let vars_path = args.vars;
    tracing::info!("load variables from: {}", vars_path.display());
    let mut vars: HashMap<String, String> =
        serde_json::from_str(&tokio::fs::read_to_string(&vars_path).await?)?;
    tracing::info!("variables loaded");
    tracing::debug!("variables: {:?}", vars);
    let variable_update_queries: Vec<(String, String)> =
        args.keyvalues.into_iter().tuples().collect();
    tracing::debug!("variable update queries: {:?}", variable_update_queries);
    tracing::info!("update variables");
    for (k, v) in variable_update_queries {
        match vars.get_mut(&k) {
            Some(val) => {
                tracing::info!("update variable: {} = {} => {}", k, val, v);
                *val = v
            }
            None => {
                tracing::error!("variable {} not found in variables", k);
                return Err(anyhow::anyhow!("variable {} not found in variables", k));
            }
        };
    }
    tracing::info!("variables updated");
    //let vars_path = args.vars;
    tracing::info!("store variables in: {}", vars_path.display());
    tokio::fs::write(&vars_path, serde_json::to_string(&vars)?).await?;
    tracing::info!("variables stored");

    if args.no_sync {
        tracing::info!("no sync mode enabled, exiting");
        return Ok(());
    }

    let conf_path = args.conf;
    tracing::info!("load config from: {}", conf_path.display());
    let conf: Config = serde_json::from_str(&tokio::fs::read_to_string(&conf_path).await?)?;
    tracing::info!("config loaded");
    tracing::debug!("config: {:?}", conf);

    let token_path = conf_path.parent().unwrap().join(&conf.token_path);
    tracing::info!("load token from: {}", token_path.display());
    let token_raw: String = serde_json::from_str(&tokio::fs::read_to_string(token_path).await?)?;
    let token = AccessToken::new(token_raw);
    tracing::info!("token loaded");
    tracing::debug!("token: {:?}", token);

    tracing::info!("instantiate ideal zone and records");
    let (records_ideal, zone_ideal) = instantiate(conf.records, &vars)?;
    tracing::info!("ideal zone and records instantiated");
    tracing::debug!("ideal zone: {:?}", zone_ideal);
    tracing::debug!("ideal records: {:?}", records_ideal);

    tracing::info!("build client");
    let c = Client::new(token);
    tracing::info!("client builded");
    tracing::debug!("client: {:?}", c);

    tracing::info!("get zone id from domain: {}", conf.domain);
    let zone_node = c.get_zone_by_name(&conf.domain).await?;
    let zone_id = zone_node.id;
    tracing::info!("zone found: {}", zone_node.id);

    let mut t = 0;
    let mut zone_real = zone_node.zone.value;

    loop {
        tracing::info!("[comparison {}]", t);

        if t != 0 {
            tracing::info!("get zone information");
            zone_real = c.get_zone(&zone_id).await?.zone.value;
            tracing::info!("zone information getted");
        } else {
            tracing::info!("reuse zone information on first trial");
        };
        tracing::debug!("real zone: {:?}", zone_real);

        tracing::info!("get records information");
        let records_real = c.get_record_list(&zone_id).await?;
        tracing::info!("records information getted");

        tracing::debug!("real records: {:?}", records_real);

        tracing::info!("generate commands");
        let records_ideal = records_ideal.clone();
        let mut coms = make_diff_commands(records_real, records_ideal);
        if let Some(zone_ideal) = &zone_ideal {
            if zone_real != *zone_ideal {
                coms.push(Command::Zone(zone_ideal.clone()));
            }
        }
        let coms = coms;
        tracing::info!("commands generated");
        tracing::debug!("commands: {:?}", coms);

        if coms.is_empty() {
            tracing::info!("no change");
            break;
        }
        if t >= conf.retry {
            tracing::error!("retry limit reached");
            return Err(anyhow::anyhow!("retry limit reached"));
        }
        tracing::info!("[sync {}]", t + 1);

        tracing::debug!("executing commands");
        let result = join_all(coms.iter().map(async |com| {
            tracing::info!("execute command {}", com);
            let r = execute_command(&c, &zone_node.id, com).await;
            match &r {
                Ok(_) => tracing::info!("command {} succeeded", com),
                Err(e) => tracing::error!("command {} failed: {}", com, e),
            }
            r
        }))
        .await;
        tracing::debug!("commands executed");

        if result.into_iter().all(|r| r.is_ok()) {
            tracing::info!("all commands succeeded");
        } else {
            tracing::error!("some commands failed");
        }
        t += 1;
    }

    // for t in 0..conf.retry {
    //     tracing::info!("trial: {}", t);
    // }

    // otherwise, create and delete
    Ok(())
}

fn instantiate(
    prerecords: Vec<PreRecord>,
    variables: &HashMap<String, String>,
) -> anyhow::Result<(Vec<Record>, Option<ZoneValue>)> {
    let records = prerecords
        .into_iter()
        .map(|r| r.resolve(variables))
        .collect::<Result<Vec<_>, anyhow::Error>>()?;

    let mut zone_addr = None;
    let records = records
        .into_iter()
        .filter_map(|r| {
            if r.name == "" {
                if let RecordValue::A { data: addr } = r.value {
                    zone_addr = Some(addr);
                    return None;
                }
            }
            Some(r)
        })
        .collect();

    Ok((
        records,
        zone_addr.map(|addr| ZoneValue {
            ipv4address: Some(addr),
            ipv6prefix: None,
        }),
    ))
}

#[derive(Debug)]
enum Command {
    Zone(ZoneValue),
    Create(Record),
    Delete(RecordNode),
    Patch(RecordNode, Record),
}
impl fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Zone(z) => write!(f, "Zone <{:?}>", z),
            Command::Create(r) => write!(f, "Create Record <{:?}>", r),
            Command::Delete(rn) => write!(f, "Delete Record [{}]", rn.id),
            Command::Patch(rn, r) => write!(f, "Patch Record [{}] => <{:?}>", rn.id, r),
        }
    }
}

async fn execute_command(c: &Client, zone_id: &ZoneID, com: &Command) -> anyhow::Result<()> {
    //tracing::info!("Execute command: {:?}", com);
    match com {
        Command::Create(r) => {
            c.add_record(&zone_id, &r).await?;
        }
        Command::Delete(rn) => {
            c.delete_record(&zone_id, &rn.id).await?;
        }
        Command::Patch(rn, r) => {
            c.update_record(&zone_id, &rn.id, &r).await?;
        }
        Command::Zone(zone_value) => {
            c.update_zone(zone_id, zone_value).await?;
        }
    }
    //tracing::info!("Executed command: {:?}", com);
    Ok(())
}

fn make_diff_commands(mut l_real: Vec<RecordNode>, l_ideal: Vec<Record>) -> Vec<Command> {
    let mut coms = Vec::new();

    // ignore perfect equal
    let l_ideal = l_ideal
        .into_iter()
        .filter_map(|r| match l_real.iter().position(|rn| rn.record == r) {
            Some(i) => {
                l_real.remove(i);
                None
            }
            None => Some(r),
        })
        .collect::<Vec<_>>();

    // patch name and type equal
    let l_ideal = l_ideal
        .into_iter()
        .filter_map(|r| {
            match l_real.iter().position(|rn| {
                let rc = &rn.record;
                r.name == rc.name
                    && std::mem::discriminant(&r.value) == std::mem::discriminant(&rc.value)
            }) {
                Some(i) => {
                    let rn = l_real.remove(i);
                    coms.push(Command::Patch(rn, r));
                    None
                }
                None => Some(r),
            }
        })
        .collect::<Vec<_>>();

    // patch type equal
    let l_ideal = l_ideal
        .into_iter()
        .filter_map(|r| {
            match l_real.iter().position(|rn| {
                let rc = &rn.record;
                std::mem::discriminant(&r.value) == std::mem::discriminant(&rc.value)
            }) {
                Some(i) => {
                    // make patch here
                    let rn = l_real.remove(i);
                    coms.push(Command::Patch(rn, r));
                    None
                }
                None => Some(r),
            }
        })
        .collect::<Vec<_>>();

    l_ideal.into_iter().for_each(|r| {
        coms.push(Command::Create(r));
    });
    l_real.into_iter().for_each(|rn| {
        coms.push(Command::Delete(rn));
    });
    coms
}

#[cfg(test)]
mod tests {
    // #[test]
    // fn test_flock() {
    //     use nix::fcntl::{Flock, FlockArg, OFlag, open};
    //     use nix::sys::stat::Mode;
    //     use std::fs::File;

    //     let exe = std::env::args().nth(0).unwrap();
    //     let fd = open(exe.as_str(), OFlag::empty(), Mode::empty()).unwrap();
    //     tokio::
    //         .read(true)
    //         .write(true)
    //         .open(exe)
    //         .unwrap();

    //     let lock = Flock::lock(fd, FlockArg::LockExclusive).unwrap();
    //     assert!(lock.is_locked());
    //     drop(lock);
    // }

    #[derive(Deserialize)]
    struct Config {
        token: String,
        domain: String,
    }

    use dynv6_rs::{RecordValue, ZoneValue};
    use futures::future::join_all;
    use serde::Deserialize;

    use super::*;

    #[tokio::test]
    async fn test_make_diff_commands() {
        let conf: Config = serde_json::from_str(include_str!(".secret.json")).unwrap();

        let c = Client::new(AccessToken::new(&conf.token));

        for _t in 0..3 {
            let l_ideal: Vec<Record> = serde_json::from_str(include_str!("ideal.json")).unwrap();
            let mut zone_addr = None;
            let l_ideal = l_ideal
                .into_iter()
                .filter_map(|r| {
                    if r.name == "" {
                        if let RecordValue::A { data: addr } = r.value {
                            zone_addr = Some(addr);
                            return None;
                        }
                    }
                    Some(r)
                })
                .collect();
            // if zone_ideal.len() > 1 {
            //     panic!("There should be only one zone record (= A record for root) in ideal.json");
            // }
            let zone_ideal = ZoneValue {
                ipv4address: zone_addr,
                ipv6prefix: None,
            };
            let zone_node = c.get_zone_by_name(&conf.domain).await.unwrap();
            let l_real = c.get_record_list(&zone_node.id).await.unwrap();

            println!("l_real: {:?}", l_real);
            println!("l_ideal: {:?}", l_ideal);

            let mut coms = make_diff_commands(l_real, l_ideal);

            println!("z_real: {:?}", zone_node.zone.value);
            println!("z_ideal: {:?}", zone_ideal);
            if zone_node.zone.value != zone_ideal {
                coms.push(Command::Zone(zone_ideal));
            }
            println!("coms: {:?}", coms);

            if coms.is_empty() {
                println!("No commands to execute.");
                break;
            }

            let result = join_all(
                coms.iter()
                    .map(|com| execute_command(&c, &zone_node.id, com)),
            )
            .await;

            println!("result: {:?}", result);
        }
    }
}
