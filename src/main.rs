use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    fs::File,
    hash::Hasher,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

const BUFFER_SIZE: usize = 188 * 8;

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
        let got = input.read_exact(&mut buf)?;
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

fn main() -> anyhow::Result<()> {
    let mut buf = [0; BUFFER_SIZE];
    let mut file = File::open("/home/yesterday17/视频/2023-03-18_14-50-06.ts")?;
    let mut file = BufReader::new(file);

    let mut count = 0;
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
                count += 1;
            }

            hasher.write_u16(header.pid);
            file.seek_relative(188 - 4)?;
        }
    }

    println!("{}", serde_json::to_string_pretty(&segments)?);
    Ok(())
}

fn cut<P1, P2>(input: P1, output: P2, start: u64, end: Option<u64>) -> anyhow::Result<()>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let mut file = File::open(input.as_ref())?;
    file.seek(SeekFrom::Start(start))?;

    let mut reader: Box<dyn Read> = match end {
        Some(end) => Box::new(file.take(end - start)),
        None => Box::new(file),
    };
    let writer = &mut File::create(output.as_ref())?;
    std::io::copy(&mut reader, writer)?;

    Ok(())
}
