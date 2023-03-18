use clap::{Args, Parser};
use clap_handler::{handler, Handler};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::Hasher,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    ops::Index,
    path::{Path, PathBuf},
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

#[derive(Serialize, Deserialize)]
struct HashFile {
    file: PathBuf,
    segments: Vec<TsSegment>,
}

impl HashFile {
    fn len(&self) -> usize {
        self.segments.len()
    }

    fn iter(&self) -> std::slice::Iter<TsSegment> {
        self.segments.iter()
    }
}

impl Index<usize> for HashFile {
    type Output = TsSegment;

    fn index(&self, index: usize) -> &Self::Output {
        &self.segments[index]
    }
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

fn do_hash<P>(video: P) -> anyhow::Result<Vec<TsSegment>>
where
    P: AsRef<Path>,
{
    const BUFFER_SIZE: usize = 188 * 8;

    let mut buf = [0; BUFFER_SIZE];
    let file = File::open(video.as_ref())?;
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

    Ok(segments)
}

#[handler(HashSubcommand)]
pub fn hash_handler(me: HashSubcommand) -> anyhow::Result<()> {
    let segments = do_hash(&me.video)?;
    let result = serde_json::to_string_pretty(&HashFile {
        file: me.video,
        segments,
    })?;
    match me.output {
        Some(output_path) => {
            File::create(output_path)?.write_all(result.as_ref())?;
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
    segment: PathBuf,
}

#[handler(MatchSubcommand)]
pub fn handle_match(me: MatchSubcommand) -> anyhow::Result<()> {
    let segment_hashes = do_hash(&me.segment)?;
    if segment_hashes.len() > 1 {
        panic!("Error: too many segments");
    }

    let segment_hash = segment_hashes[0].hash;
    let hashes: HashFile = serde_json::from_reader(File::open(me.hashes)?)?;

    let mut result = Vec::new();
    for (index, segment) in hashes.iter().enumerate() {
        if segment.hash == segment_hash {
            result.push(index);
        }
    }

    let result = if result.len() > 1 {
        let mut new_result = Vec::new();
        let segment_length = me.segment.metadata()?.len();
        let mut segment_file = File::open(&me.segment)?;
        let mut segment_buffer = Vec::with_capacity(segment_length as usize);
        segment_file.read_exact(&mut segment_buffer)?;

        for index in result {
            let start = hashes[index].offset;
            let end = if index + 1 == hashes.len() {
                hashes.file.metadata()?.len()
            } else {
                hashes[index + 1].offset
            };

            if segment_length != end - start {
                continue;
            }

            let mut file = File::open(&hashes.file)?;
            file.seek(SeekFrom::Start(start))?;
            let mut buffer = Vec::with_capacity(segment_length as usize);
            file.read_exact(&mut buffer)?;
            if buffer == segment_buffer {
                new_result.push(index);
            }
        }
        new_result
    } else {
        result
    };

    if result.is_empty() {
        println!("Error: segment not found");
    } else {
        let mut counter = 0;

        for index in result {
            counter += 1;
            println!("#{counter}:");
            if index > 0 {
                println!(
                    "Previous block: mtf cut --from={} --to={} <video> <output>",
                    hashes[index - 1].offset,
                    hashes[index].offset
                );
            }
            if index < hashes.len() - 1 {
                println!(
                    "Current block:  mtf cut --from={} --to={} <video> <output>",
                    hashes[index].offset,
                    hashes[index + 1].offset
                );
            } else {
                println!(
                    "Current block:  mtf cut --from={} <video> <output>",
                    hashes[index].offset
                );
            }
            if index < hashes.len() - 2 {
                println!(
                    "Next block:     mtf cut --from={} --to={} <video> <output>",
                    hashes[index + 1].offset,
                    hashes[index + 2].offset
                );
            } else if index < hashes.len() - 1 {
                println!(
                    "Next block:     mtf cut --from={} <video> <output>",
                    hashes[index + 1].offset
                );
            }
            println!();
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

    video: PathBuf,
    output: PathBuf,
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
