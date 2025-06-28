use std::{
    collections::HashMap,
    error::Error,
    net::{Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

use dynv6_rs::RecordValue;
use serde::Deserialize;

pub trait Resolvable {
    type Output;
    fn resolve(self, variables: &HashMap<String, String>) -> anyhow::Result<Self::Output>;
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct Precursor<T> {
    expr: String,
    #[serde(skip)]
    _ph: std::marker::PhantomData<T>,
}
impl<T: FromStr> Resolvable for Precursor<T>
where
    T::Err: Error + Send + Sync + 'static,
{
    type Output = T;

    fn resolve(self, variables: &HashMap<String, String>) -> anyhow::Result<Self::Output> {
        let replaced = replace(self.expr, variables);

        Ok(<T as FromStr>::from_str(&replaced)?)
    }
}
fn replace(expr: String, variables: &HashMap<String, String>) -> String {
    //tracing::debug!("expr: {}", expr);
    let mut replaced = expr;
    for _t in 0..5 {
        for (key, value) in variables {
            let var = format!("${{{}}}", key);
            replaced = replaced.replace(&var, value);
        }
    }
    //tracing::debug!("replaced: {}", replaced);
    replaced
}

#[derive(Debug, Deserialize)]
pub enum PreRecordValue {
    A {
        addr: Precursor<Ipv4Addr>,
    },
    AAAA {
        addr: Precursor<Ipv6Addr>,
    },
    CNAME {
        domain: Precursor<String>,
        //expandedData: String,
    },
    SRV {
        domain: Precursor<String>,
        priority: Precursor<u16>,
        weight: Precursor<u16>,
        port: Precursor<u16>,
    },
    TXT {
        data: Precursor<String>,
    },
}
impl Resolvable for PreRecordValue {
    type Output = RecordValue;

    fn resolve(self, variables: &HashMap<String, String>) -> anyhow::Result<Self::Output> {
        match self {
            PreRecordValue::A { addr } => {
                let addr = addr.resolve(variables)?;
                Ok(RecordValue::A { data: addr })
            }
            PreRecordValue::AAAA { addr } => {
                let addr = addr.resolve(variables)?;
                Ok(RecordValue::AAAA { data: addr })
            }
            PreRecordValue::CNAME { domain } => {
                let domain = domain.resolve(variables)?;
                Ok(RecordValue::CNAME { data: domain })
            }
            PreRecordValue::SRV {
                domain,
                priority,
                weight,
                port,
            } => {
                let data = domain.resolve(variables)?;
                let priority = priority.resolve(variables)?;
                let weight = weight.resolve(variables)?;
                let port = port.resolve(variables)?;
                Ok(RecordValue::SRV {
                    data,
                    priority,
                    weight,
                    port,
                })
            }
            PreRecordValue::TXT { data } => {
                let data = data.resolve(variables)?;
                Ok(RecordValue::TXT { data })
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PreRecord {
    name: Precursor<String>,
    #[serde(flatten)]
    value: PreRecordValue,
}
impl Resolvable for PreRecord {
    type Output = dynv6_rs::Record;

    fn resolve(self, variables: &HashMap<String, String>) -> anyhow::Result<Self::Output> {
        let name = self.name.resolve(variables)?;
        let value = self.value.resolve(variables)?;

        Ok(dynv6_rs::Record { name, value })
    }
}
