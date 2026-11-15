//! Minimal Valve VPK (v1/v2) directory reader: lists files and extracts single entries.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SIGNATURE: u32 = 0x55AA_1234;

#[derive(Clone, Debug)]
pub struct Entry {
    /// Lowercase, forward-slash path, e.g. `models/weapons/v_rifle.mdl`.
    pub path: String,
    pub crc: u32,
    archive: u16,
    preload_pos: u64,
    preload_len: u16,
    offset: u32,
    len: u32,
}

impl Entry {
    pub fn size(&self) -> u64 {
        self.preload_len as u64 + self.len as u64
    }
}

pub struct Vpk {
    pub path: PathBuf,
    data_base: u64,
    pub entries: Vec<Entry>,
}

fn rd_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}
fn rd_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn rd_cstr(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<String> {
    buf.clear();
    r.read_until(0, buf)?;
    if buf.last() != Some(&0) {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated VPK tree",
        ));
    }
    buf.pop();
    Ok(String::from_utf8_lossy(buf).into_owned())
}

impl Vpk {
    pub fn open(path: &Path) -> io::Result<Vpk> {
        let mut r = BufReader::with_capacity(64 * 1024, File::open(path)?);
        if rd_u32(&mut r)? != SIGNATURE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a VPK file"));
        }
        let version = rd_u32(&mut r)?;
        let tree_size = rd_u32(&mut r)? as u64;
        let header = match version {
            1 => 12,
            2 => {
                r.seek_relative(16)?;
                28
            }
            v => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported VPK version {v}"),
                ));
            }
        };
        let mut entries = Vec::new();
        let mut buf = Vec::new();
        let mut pos = header as u64;
        loop {
            let ext = rd_cstr(&mut r, &mut buf)?;
            pos += ext.len() as u64 + 1;
            if ext.is_empty() {
                break;
            }
            loop {
                let dir = rd_cstr(&mut r, &mut buf)?;
                pos += dir.len() as u64 + 1;
                if dir.is_empty() {
                    break;
                }
                loop {
                    let name = rd_cstr(&mut r, &mut buf)?;
                    pos += name.len() as u64 + 1;
                    if name.is_empty() {
                        break;
                    }
                    let crc = rd_u32(&mut r)?;
                    let preload_len = rd_u16(&mut r)?;
                    let archive = rd_u16(&mut r)?;
                    let offset = rd_u32(&mut r)?;
                    let len = rd_u32(&mut r)?;
                    let _term = rd_u16(&mut r)?;
                    pos += 18;
                    let preload_pos = pos;
                    r.seek_relative(preload_len as i64)?;
                    pos += preload_len as u64;
                    let mut full = String::with_capacity(dir.len() + name.len() + ext.len() + 2);
                    if dir != " " {
                        full.push_str(dir.trim_matches('/'));
                        full.push('/');
                    }
                    full.push_str(&name);
                    if ext != " " {
                        full.push('.');
                        full.push_str(&ext);
                    }
                    full.make_ascii_lowercase();
                    entries.push(Entry {
                        path: full.replace('\\', "/"),
                        crc,
                        archive,
                        preload_pos,
                        preload_len,
                        offset,
                        len,
                    });
                }
            }
            if pos > header as u64 + tree_size + 1 {
                break;
            }
        }
        Ok(Vpk {
            path: path.to_path_buf(),
            data_base: header as u64 + tree_size,
            entries,
        })
    }

    pub fn find(&self, path: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.path.eq_ignore_ascii_case(path))
    }

    pub fn read(&self, e: &Entry) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(e.size() as usize);
        let mut f = File::open(&self.path)?;
        if e.preload_len > 0 {
            f.seek(SeekFrom::Start(e.preload_pos))?;
            out.resize(e.preload_len as usize, 0);
            f.read_exact(&mut out)?;
        }
        if e.len > 0 {
            let (mut src, off) = if e.archive == 0x7fff {
                (f, self.data_base + e.offset as u64)
            } else {
                // Multi-part pack: foo_dir.vpk -> foo_000.vpk
                let s = self.path.to_string_lossy();
                let base = s.strip_suffix("_dir.vpk").unwrap_or(&s);
                (
                    File::open(format!("{base}_{:03}.vpk", e.archive))?,
                    e.offset as u64,
                )
            };
            src.seek(SeekFrom::Start(off))?;
            let start = out.len();
            out.resize(start + e.len as usize, 0);
            src.read_exact(&mut out[start..])?;
        }
        Ok(out)
    }

    pub fn read_path(&self, path: &str) -> Option<Vec<u8>> {
        self.find(path).and_then(|e| self.read(e).ok())
    }
}
