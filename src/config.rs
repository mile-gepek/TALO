use std::{
    fs, io,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use arc_swap::ArcSwap;
use thiserror::Error;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn duration_as_seconds<S: Serializer>(
    duration: &Duration,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let seconds = duration.as_secs_f32();
    serializer.serialize_f32(seconds)
}

fn duration_from_seconds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    let seconds = f32::deserialize(deserializer)?;
    Ok(Duration::from_secs_f32(seconds))
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfterResumeBehavior {
    /// Reschedule the break after resuming the system.
    Reschedule,
    /// Keep the break's remaining time, clamped to the lower bound [`min_wait_after_resume`].
    ///
    /// The system does not remove scheduled breaks on paused.
    /// Instead, it saves the timestamp of when it was paused,
    /// and uses it to calculate the time remaining until the break's deadline.
    /// That remaining time is clamped to [`min_wait_after_resume`],
    /// and becomes the delay until the break runs.
    ///
    /// [`min_wait_after_resume`]: AfterResumeConfig::min_wait_after_resume
    #[default]
    KeepScheduledBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterResumeConfig {
    #[serde(alias = "mode")]
    pub behavior: AfterResumeBehavior,
    /// Lower bound for the delay before firing a pending break
    /// after resuming when [`AfterResumeBehavior::KeepTimeRemaining`] is used.
    #[serde(
        serialize_with = "duration_as_seconds",
        deserialize_with = "duration_from_seconds"
    )]
    #[serde(alias = "minimum-delay")]
    pub min_wait_after_resume: Duration,
}

impl Default for AfterResumeConfig {
    fn default() -> Self {
        Self {
            behavior: AfterResumeBehavior::default(),
            min_wait_after_resume: Duration::from_secs(15),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakConfig {
    #[serde(
        serialize_with = "duration_as_seconds",
        deserialize_with = "duration_from_seconds"
    )]
    pub short_break_interval: Duration,
    #[serde(
        serialize_with = "duration_as_seconds",
        deserialize_with = "duration_from_seconds"
    )]
    pub short_break_duration: Duration,
    #[serde(
        serialize_with = "duration_as_seconds",
        deserialize_with = "duration_from_seconds"
    )]
    pub long_break_duration: Duration,
    pub long_break_interval: usize,
    #[serde(alias = "after-resume")]
    pub after_resume_config: AfterResumeConfig,
}

impl BreakConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for BreakConfig {
    fn default() -> Self {
        Self {
            short_break_duration: Duration::from_secs(15),
            short_break_interval: Duration::from_mins(15),
            long_break_duration: Duration::from_secs(60),
            long_break_interval: 2,
            after_resume_config: AfterResumeConfig::default(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Serialize(#[from] toml::ser::Error),
    #[error(transparent)]
    Deserialize(#[from] toml::de::Error),
    #[error("no config directory available")]
    NoConfigDir,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(alias = "breaks")]
    pub breaks: BreakConfig,
}

impl Config {
    pub fn path() -> Result<PathBuf, ConfigError> {
        let mut config_dir = dirs::config_local_dir().ok_or(ConfigError::NoConfigDir)?;
        config_dir.push("talo/config.toml");
        Ok(config_dir)
    }

    fn read() -> Result<Self, ConfigError> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(raw) => Ok(toml::from_str(&raw)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let config = Self::default();
                config.save()?;
                Ok(config)
            }
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn reload() -> Result<Self, ConfigError> {
        let config = Self::read()?;
        CONFIG.store(Arc::new(config));
        Ok(config)
    }
}

pub fn update(mutate: impl FnOnce(&mut Config)) -> Result<Config, ConfigError> {
    let current = *CONFIG.load_full();
    let mut next = current;
    mutate(&mut next);

    if next != current {
        next.save()?;
        CONFIG.store(Arc::new(next));
    }

    tracing::debug!("Saving new config: {next:?}");

    Ok(next)
}

pub static CONFIG: LazyLock<ArcSwap<Config>> = LazyLock::new(|| {
    let config = Config::read()
        .inspect_err(|err| tracing::error!("Error loading config: '{err}'\nUsing default config"))
        .unwrap_or_default();
    ArcSwap::from_pointee(config)
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Only runs when a scratch `XDG_CONFIG_HOME` is set, so a plain
    /// `cargo test` never touches the user's real config.
    #[test]
    fn save_and_read_round_trip_on_disk() {
        if std::env::var_os("XDG_CONFIG_HOME").is_none() {
            return;
        }
        let config = Config::default();
        config.save().expect("save");
        let read = Config::read().expect("read");
        assert_eq!(
            config.breaks.long_break_interval,
            read.breaks.long_break_interval
        );
        assert_eq!(
            config.breaks.short_break_duration,
            read.breaks.short_break_duration
        );
        assert_eq!(
            config.breaks.after_resume_config.min_wait_after_resume,
            read.breaks.after_resume_config.min_wait_after_resume
        );
    }

    #[test]
    fn config_round_trips_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).expect("serialize");
        println!("{text}");
        let parsed: Config = toml::from_str(&text).expect("deserialize");

        let before = config.breaks;
        let after = parsed.breaks;
        assert_eq!(before.short_break_duration, after.short_break_duration);
        assert_eq!(before.short_break_interval, after.short_break_interval);
        assert_eq!(before.long_break_duration, after.long_break_duration);
        assert_eq!(before.long_break_interval, after.long_break_interval);
        assert_eq!(
            before.after_resume_config.behavior,
            after.after_resume_config.behavior
        );
        assert_eq!(
            before.after_resume_config.min_wait_after_resume,
            after.after_resume_config.min_wait_after_resume
        );
    }
}
