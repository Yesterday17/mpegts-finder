use clap::{Args, Parser};
use clap_handler::{handler, Handler};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::Hasher,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

struct MpegtsHeader {
    is_start: bool,
    pid: u16,
}

impl MpegtsHeader {
    pub fn new<R>(input: &mut R) -> anyhow::Result<Self>
    where
        R: Read + Seek,
    {
        let mut buf = [0u8; 4];
        input.read_exact(&mut buf)?;
        let header = u32::from_be_bytes(buf);
        assert!(header & 0xff000000 == 0x47000000, "sync byte not found");

        let is_start = (header & 0x400000) != 0;
        let pid = ((header & 0x1fff00) >> 8) as u16;

        Ok(Self { is_start, pid })
    }
}

#[derive(Serialize, Deserialize)]
struct TsSegment {
    hash: u64,
    offset: u64,
}

#[derive(Parser, Handler, Debug, Clone)]
#[clap(name = "mpegts-finder", author)]
struct MTF {
    #[clap(subcommand)]
    subcommand: Subcommand,
}

#[derive(Parser, Handler, Debug, Clone)]
pub enum Subcommand {
    Hash(HashSubcommand),
    Cut(CutSubcommand),
    Match(MatchSubcommand),
}

#[derive(Args, Debug, Clone)]
pub struct HashSubcommand {
    #[clap(short, long)]
    output: Option<PathBuf>,

    video: PathBuf,
}

#[handler(HashSubcommand)]
pub fn hash_handler(me: HashSubcommand) -> anyhow::Result<()> {
    const BUFFER_SIZE: usize = 188 * 8;

    let mut buf = [0; BUFFER_SIZE];
    let file = File::open(me.video)?;
    let mut file = BufReader::new(file);

    let mut hasher = DefaultHasher::new();
    let mut prev_segment_offset: Option<u64> = None;

    let mut segments = Vec::new();

    loop {
        // find the first 0x47
        let read = file.read(&mut buf)?;
        if read == 0 {
            // EOF
            let prev_segment_offset = prev_segment_offset.unwrap();
            let hash = hasher.finish();
            segments.push(TsSegment {
                hash,
                offset: prev_segment_offset,
            });
            break;
        }

        let got = &buf[0..read];
        if let Some(position) = got.iter().position(|b| *b == 0x47) {
            // sync byte found, seek back for file
            file.seek_relative(-(read as i64 - position as i64))?;

            let header = MpegtsHeader::new(&mut file)?;
            if header.pid == 0 && header.is_start {
                // found segment start
                if let Some(prev_segment_offset) = prev_segment_offset {
                    let hash = hasher.finish();
                    hasher = DefaultHasher::new();
                    segments.push(TsSegment {
                        hash,
                        offset: prev_segment_offset,
                    });
                }

                let offset = file.stream_position()? - 4;
                prev_segment_offset = Some(offset);
            }

            hasher.write_u16(header.pid);
            file.seek_relative(188 - 4)?;
        }
    }

    let result = serde_json::to_string_pretty(&segments)?;
    match me.output {
        Some(output_path) => {
            File::create(output_path)?.write_all(buf.as_ref())?;
        }
        None => {
            println!("{result}");
        }
    }
    Ok(())
}

#[derive(Args, Debug, Clone)]
pub struct MatchSubcommand {
    hashes: PathBuf,
    hash: String,
}

#[handler(MatchSubcommand)]
pub fn handle_match(me: MatchSubcommand) -> anyhow::Result<()> {
    let hashes: Vec<TsSegment> = serde_json::from_reader(File::open(me.hashes)?)?;
    let hash = u64::from_str_radix(&me.hash, 10)?;

    let mut result = Vec::new();
    for (index, segment) in hashes.iter().enumerate() {
        if segment.hash == hash {
            result.push(index);
        }
    }

    if result.len() > 1 {
        unimplemented!();
    }

    if result.is_empty() {
        println!("Error: segment not found");
    } else {
        let index = result[0];
        if index > 0 {
            println!("[-1] {}", hashes[index - 1].offset);
        }
        println!("[+0] {}", hashes[index].offset);
        if index < hashes.len() - 1 {
            println!("[+1] {}", hashes[index + 1].offset);
        }
        if index < hashes.len() - 2 {
            println!("[+2] {}", hashes[index + 2].offset);
        }
    }

    Ok(())
}

#[derive(Args, Debug, Clone)]
pub struct CutSubcommand {
    #[clap(long)]
    from: u64,
    #[clap(long)]
    to: Option<u64>,

    #[clap(short, long)]
    output: PathBuf,
    video: PathBuf,
}

#[handler(CutSubcommand)]
fn handle_cut(me: CutSubcommand) -> anyhow::Result<()> {
    let mut file = File::open(me.video)?;
    file.seek(SeekFrom::Start(me.from))?;

    let mut reader: Box<dyn Read> = match me.to {
        Some(end) => Box::new(file.take(end - me.from)),
        None => Box::new(file),
    };
    let writer = &mut File::create(me.output)?;
    std::io::copy(&mut reader, writer)?;

    Ok(())
}

fn main() -> anyhow::Result<()> {
    MTF::parse().run()
}
