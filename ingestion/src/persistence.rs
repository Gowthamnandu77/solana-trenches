use serde_json::Value;
use std::{io, path::Path};
use tokio::{
    fs::{create_dir_all, File, OpenOptions},
    io::{AsyncWriteExt, BufWriter},
};

pub struct Persistence {
    files: [BufWriter<File>; 6],
}
#[derive(Clone, Copy)]
pub enum Stream {
    Events = 0,
    Pools = 1,
    Unknown = 2,
    Momentum = 3,
    Metrics = 4,
    Features = 5,
}
impl Persistence {
    pub async fn open(directory: &Path) -> io::Result<Self> {
        create_dir_all(directory).await?;
        async fn file(dir: &Path, name: &str) -> io::Result<BufWriter<File>> {
            Ok(BufWriter::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join(name))
                    .await?,
            ))
        }
        Ok(Self {
            files: [
                file(directory, "launch_events_v15.jsonl").await?,
                file(directory, "new_launches_v15.jsonl").await?,
                file(directory, "unknown_instructions_v15.jsonl").await?,
                file(directory, "momentum_snapshots_v15.jsonl").await?,
                file(directory, "runtime_metrics_v15.jsonl").await?,
                file(directory, "features_v15.jsonl").await?,
            ],
        })
    }
    pub async fn write(&mut self, stream: Stream, value: &Value) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        let file = &mut self.files[stream as usize];
        file.write_all(&bytes).await?;
        file.flush().await
    }
    pub async fn flush(&mut self) -> io::Result<()> {
        for file in &mut self.files {
            file.flush().await?;
        }
        Ok(())
    }
}
