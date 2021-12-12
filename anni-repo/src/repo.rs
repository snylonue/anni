use serde::{Serialize, Deserialize};
use std::str::FromStr;
use anni_derive::FromFile;
use crate::prelude::*;

#[derive(Serialize, Deserialize, FromFile)]
pub struct Repository {
    repo: RepositoryInner,
}

#[derive(Serialize, Deserialize)]
struct RepositoryInner {
    name: String,
    edition: String,

    cover: Option<AssetSetting>,
    lyric: Option<AssetSetting>,
}

#[derive(Serialize, Deserialize)]
pub struct AssetSetting {
    pub enable: bool,
    root: Option<String>,
}

impl FromStr for Repository {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val: Repository = toml::from_str(s)
            .map_err(|e| Error::TomlParseError {
                target: "Repository",
                input: s.to_string(),
                err: e,
            })?;
        Ok(val)
    }
}

impl ToString for Repository {
    fn to_string(&self) -> String {
        toml::to_string(&self).unwrap()
    }
}

impl Repository {
    pub fn name(&self) -> &str {
        self.repo.name.as_ref()
    }

    pub fn edition(&self) -> &str {
        self.repo.edition.as_ref()
    }

    pub fn cover(&self) -> Option<&AssetSetting> {
        self.repo.cover.as_ref()
    }

    pub fn lyric(&self) -> Option<&AssetSetting> {
        self.repo.lyric.as_ref()
    }
}

impl AssetSetting {
    pub fn root(&self) -> Option<&str> {
        self.root.as_ref().map(|r| r.as_ref())
    }
}
