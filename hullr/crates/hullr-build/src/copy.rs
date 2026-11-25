//! `COPY` / `ADD` mechanics: source resolution (globs, `.dockerignore`,
//! heredocs), destination semantics, tar auto-extract for `ADD`, and a
//! content digest of the sources for the build cache.

use crate::fsutil::{clean_abs, resolve_in_root};
use crate::ignore::{glob_component, has_glob_meta, DockerIgnore};
use hullr_core::{Context, Error, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// A heredoc usable as a COPY source: `(name, content)`.
pub type HeredocSrc = (String, String);

pub struct CopyCtx<'a> {
    /// Build context directory, or the rootfs of a `--from` source.
    pub src_root: &'a Path,
    /// Only the build context is filtered by `.dockerignore`.
    pub ignore: Option<&'a DockerIgnore>,
    /// The working rootfs being modified (unused for digests).
    pub dest_root: &'a Path,
    /// Current container WORKDIR (absolute).
    pub workdir: &'a str,
    pub is_add: bool,
}

#[derive(Debug)]
enum Src {
    Path { abs: PathBuf, rel: String },
    Heredoc { name: String, content: String },
}

fn heredoc_name(src: &str) -> Option<String> {
    let s = src.strip_prefix("<<")?;
    let s = s.strip_prefix('-').unwrap_or(s);
    Some(s.trim_matches(['"', '\'']).to_string())
}

fn is_ignored(ctx: &CopyCtx, rel: &str) -> bool {
    ctx.ignore.map(|i| i.is_ignored(rel)).unwrap_or(false)
}

fn resolve_sources(ctx: &CopyCtx, sources: &[String], heredocs: &[HeredocSrc]) -> Result<Vec<Src>> {
    let mut out: Vec<Src> = Vec::new();
    for s in sources {
        if let Some(name) = heredoc_name(s) {
            let (n, c) = heredocs
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| Error::Build(format!("heredoc '{name}' not found")))?;
            out.push(Src::Heredoc { name: n.clone(), content: c.clone() });
            continue;
        }
        if ctx.is_add && (s.starts_with("http://") || s.starts_with("https://")) {
            return Err(Error::Unsupported(format!("ADD from URL is not supported ({s})")));
        }
        let pat = clean_abs(s);
        let pat = pat.trim_start_matches('/');
        let mut rels: Vec<String> = Vec::new();
        if has_glob_meta(pat) {
            let mut cur: Vec<String> = vec![String::new()];
            for comp in pat.split('/').filter(|c| !c.is_empty()) {
                let mut next = Vec::new();
                for dir in &cur {
                    if has_glob_meta(comp) {
                        let host = resolve_in_root(ctx.src_root, dir, true)?;
                        if let Ok(rd) = std::fs::read_dir(&host) {
                            let mut names: Vec<String> =
                                rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
                            names.sort();
                            for n in names {
                                if glob_component(comp, &n) {
                                    next.push(if dir.is_empty() { n } else { format!("{dir}/{n}") });
                                }
                            }
                        }
                    } else {
                        next.push(if dir.is_empty() { comp.to_string() } else { format!("{dir}/{comp}") });
                    }
                }
                cur = next;
            }
            rels = cur;
        } else {
            rels.push(pat.to_string());
        }
        let mut found = false;
        for rel in rels {
            let abs = resolve_in_root(ctx.src_root, &rel, true)?;
            if std::fs::symlink_metadata(&abs).is_err() || (is_ignored(ctx, &rel) && !rel.is_empty()) {
                continue;
            }
            found = true;
            out.push(Src::Path { abs, rel });
        }
        if !found {
            return Err(Error::Build(format!("COPY/ADD source '{s}' not found (or excluded by .dockerignore)")));
        }
    }
    Ok(out)
}

/// Visit every entry below `abs` (depth first, sorted), honoring ignores.
fn walk_filtered(
    ctx: &CopyCtx,
    abs: &Path,
    rel: &str,
    f: &mut dyn FnMut(&Path, &str, &std::fs::Metadata) -> Result<()>,
) -> Result<()> {
    let mut names: Vec<_> = std::fs::read_dir(abs)
        .context(format!("read {}", abs.display()))?
        .flatten()
        .map(|e| e.file_name())
        .collect();
    names.sort();
    for n in names {
        let p = abs.join(&n);
        let r = if rel.is_empty() { n.to_string_lossy().into_owned() } else { format!("{rel}/{}", n.to_string_lossy()) };
        if is_ignored(ctx, &r) {
            continue;
        }
        let md = std::fs::symlink_metadata(&p)?;
        f(&p, &r, &md)?;
        if md.is_dir() {
            walk_filtered(ctx, &p, &r, f)?;
        }
    }
    Ok(())
}

/// Hex digest over the paths, modes and contents of everything the
/// instruction would copy.
pub fn source_digest(ctx: &CopyCtx, sources: &[String], heredocs: &[HeredocSrc]) -> Result<String> {
    let srcs = resolve_sources(ctx, sources, heredocs)?;
    let mut h = Sha256::new();
    let mut hash_entry = |p: &Path, rel: &str, md: &std::fs::Metadata| -> Result<()> {
        h.update(rel.as_bytes());
        h.update([0]);
        h.update((md.mode() & 0o7777).to_le_bytes());
        if md.file_type().is_symlink() {
            h.update(b"L");
            h.update(std::fs::read_link(p)?.as_os_str().as_encoded_bytes());
        } else if md.is_file() {
            h.update(b"F");
            let mut f = std::fs::File::open(p).context(format!("read {}", p.display()))?;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
        } else {
            h.update(b"D");
        }
        h.update([0xff]);
        Ok(())
    };
    for s in &srcs {
        match s {
            Src::Heredoc { name, content } => {
                let mut hh = Sha256::new();
                hh.update(name.as_bytes());
                hh.update(content.as_bytes());
                // fold through the same hasher via a pseudo entry
                let key = hex::encode(hh.finalize());
                hash_entry_heredoc(&mut hash_entry, &key)?;
            }
            Src::Path { abs, rel } => {
                let md = std::fs::metadata(abs)?;
                hash_entry(abs, rel, &md)?;
                if md.is_dir() {
                    walk_filtered(ctx, abs, rel, &mut hash_entry)?;
                }
            }
        }
    }
    Ok(hex::encode(h.finalize()))
}

fn hash_entry_heredoc(f: &mut dyn FnMut(&Path, &str, &std::fs::Metadata) -> Result<()>, key: &str) -> Result<()> {
    // Represent the heredoc as a pseudo directory entry named by its digest.
    let md = std::fs::metadata(".")?;
    f(Path::new("."), &format!("<<heredoc:{key}"), &md)
}

/// Perform the copy. Returns every host path created or written under
/// `dest_root` (for `--chown`).
pub fn do_copy(
    ctx: &CopyCtx,
    sources: &[String],
    dest: &str,
    heredocs: &[HeredocSrc],
    chmod: Option<u32>,
) -> Result<Vec<PathBuf>> {
    let srcs = resolve_sources(ctx, sources, heredocs)?;
    let dest_abs = if dest.starts_with('/') { clean_abs(dest) } else { clean_abs(&format!("{}/{}", ctx.workdir, dest)) };
    let dir_hint = dest.ends_with('/') || dest == "." || dest.ends_with("/.") || dest == "..";
    if srcs.len() > 1 && !dir_hint {
        return Err(Error::Build(
            "when copying multiple sources the destination must be a directory ending with '/'".into(),
        ));
    }
    let dest_host = resolve_in_root(ctx.dest_root, &dest_abs, true)?;
    let dest_is_dir = dir_hint || dest_host.is_dir();
    let mut touched = Vec::new();

    let ensure_dir = |p: &Path, touched: &mut Vec<PathBuf>| -> Result<()> {
        if !p.exists() {
            // record each newly created ancestor under dest_root
            let mut missing = Vec::new();
            let mut cur = p;
            while !cur.exists() && cur.starts_with(ctx.dest_root) && cur != ctx.dest_root {
                missing.push(cur.to_path_buf());
                match cur.parent() {
                    Some(par) => cur = par,
                    None => break,
                }
            }
            std::fs::create_dir_all(p).context(format!("create {}", p.display()))?;
            touched.extend(missing.into_iter().rev());
        }
        Ok(())
    };

    for s in &srcs {
        match s {
            Src::Heredoc { name, content } => {
                let target = if dest_is_dir { dest_host.join(name) } else { dest_host.clone() };
                if let Some(par) = target.parent() {
                    ensure_dir(par, &mut touched)?;
                }
                write_file(&target, content.as_bytes(), 0o644, chmod)?;
                touched.push(target);
            }
            Src::Path { abs, .. } => {
                let md = std::fs::metadata(abs)?;
                if md.is_dir() {
                    ensure_dir(&dest_host, &mut touched)?;
                    if let Some(m) = chmod {
                        let _ = std::fs::set_permissions(&dest_host, std::fs::Permissions::from_mode(m));
                    }
                    let rel_root = match s {
                        Src::Path { rel, .. } => rel.clone(),
                        _ => String::new(),
                    };
                    copy_dir(ctx, abs, &rel_root, &dest_host, chmod, &mut touched)?;
                } else {
                    if ctx.is_add && is_tar(abs) {
                        ensure_dir(&dest_host, &mut touched)?;
                        extract_tar(abs, &dest_host)?;
                        continue;
                    }
                    let base = abs.file_name().map(|n| n.to_owned()).unwrap_or_default();
                    let target = if dest_is_dir { dest_host.join(&base) } else { dest_host.clone() };
                    if let Some(par) = target.parent() {
                        ensure_dir(par, &mut touched)?;
                    }
                    copy_file(abs, &target, chmod)?;
                    touched.push(target);
                }
            }
        }
    }
    Ok(touched)
}

fn write_file(target: &Path, data: &[u8], mode: u32, chmod: Option<u32>) -> Result<()> {
    clear_target(target)?;
    std::fs::write(target, data).context(format!("write {}", target.display()))?;
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(chmod.unwrap_or(mode)))?;
    Ok(())
}

/// Remove a symlink or file at `target` so writes never go through it.
fn clear_target(target: &Path) -> Result<()> {
    if let Ok(md) = std::fs::symlink_metadata(target) {
        if md.is_dir() {
            return Err(Error::Build(format!("cannot overwrite directory {} with a file", target.display())));
        }
        std::fs::remove_file(target)?;
    }
    Ok(())
}

fn copy_file(src: &Path, target: &Path, chmod: Option<u32>) -> Result<()> {
    clear_target(target)?;
    std::fs::copy(src, target).context(format!("copy {} to {}", src.display(), target.display()))?;
    if let Some(m) = chmod {
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(m))?;
    }
    Ok(())
}

fn copy_dir(
    ctx: &CopyCtx,
    src: &Path,
    rel: &str,
    dst: &Path,
    chmod: Option<u32>,
    touched: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(src)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let r = if rel.is_empty() { name.to_string_lossy().into_owned() } else { format!("{rel}/{}", name.to_string_lossy()) };
        if is_ignored(ctx, &r) {
            continue;
        }
        let from = e.path();
        let to = dst.join(&name);
        let md = std::fs::symlink_metadata(&from)?;
        if md.file_type().is_symlink() {
            if std::fs::symlink_metadata(&to).is_ok() {
                if to.is_dir() && !std::fs::symlink_metadata(&to)?.file_type().is_symlink() {
                    std::fs::remove_dir_all(&to)?;
                } else {
                    std::fs::remove_file(&to)?;
                }
            }
            symlink(std::fs::read_link(&from)?, &to)?;
        } else if md.is_dir() {
            if !to.is_dir() {
                clear_target(&to)?;
                std::fs::create_dir(&to)?;
            }
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(chmod.unwrap_or(md.mode() & 0o7777)))?;
            touched.push(to.clone());
            copy_dir(ctx, &from, &r, &to, chmod, touched)?;
            continue;
        } else if md.is_file() {
            copy_file(&from, &to, chmod)?;
        } else {
            continue;
        }
        touched.push(to);
    }
    Ok(())
}

fn is_tar(p: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(p) else { return false };
    let mut head = [0u8; 512];
    let mut n = 0;
    while n < 2 {
        match f.read(&mut head[n..2]) {
            Ok(0) | Err(_) => return false,
            Ok(k) => n += k,
        }
    }
    let gz = head[0] == 0x1f && head[1] == 0x8b;
    let mut buf = [0u8; 512];
    let got = if gz {
        let Ok(f) = std::fs::File::open(p) else { return false };
        read_full(&mut flate2::read::GzDecoder::new(f), &mut buf)
    } else {
        let Ok(mut f) = std::fs::File::open(p) else { return false };
        read_full(&mut f, &mut buf)
    };
    got == 512 && &buf[257..262] == b"ustar"
}

fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    n
}

fn extract_tar(p: &Path, dest: &Path) -> Result<()> {
    let mut f = std::fs::File::open(p)?;
    let mut magic = [0u8; 2];
    let _ = f.read(&mut magic);
    let f = std::fs::File::open(p)?;
    let reader: Box<dyn Read> = if magic == [0x1f, 0x8b] { Box::new(flate2::read::GzDecoder::new(f)) } else { Box::new(f) };
    let mut a = tar::Archive::new(reader);
    a.set_preserve_permissions(true);
    a.unpack(dest).context(format!("extract {}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T {
        _td: tempfile::TempDir,
        ctx_dir: PathBuf,
        root: PathBuf,
    }
    fn setup() -> T {
        let td = tempfile::tempdir().unwrap();
        let ctx_dir = td.path().join("ctx");
        let root = td.path().join("root");
        std::fs::create_dir_all(&ctx_dir).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        T { _td: td, ctx_dir, root }
    }
    fn put(base: &Path, rel: &str, content: &str) {
        let p = base.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    fn cx<'a>(t: &'a T, ig: Option<&'a DockerIgnore>, wd: &'a str, add: bool) -> CopyCtx<'a> {
        CopyCtx { src_root: &t.ctx_dir, ignore: ig, dest_root: &t.root, workdir: wd, is_add: add }
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn file_to_file_dir_and_workdir() {
        let t = setup();
        put(&t.ctx_dir, "a.txt", "A");
        let c = cx(&t, None, "/app", false);
        do_copy(&c, &s(&["a.txt"]), "/etc/renamed", &[], None).unwrap();
        assert_eq!(std::fs::read_to_string(t.root.join("etc/renamed")).unwrap(), "A");
        do_copy(&c, &s(&["a.txt"]), "/opt/dir/", &[], None).unwrap();
        assert_eq!(std::fs::read_to_string(t.root.join("opt/dir/a.txt")).unwrap(), "A");
        do_copy(&c, &s(&["a.txt"]), "rel.txt", &[], None).unwrap();
        assert_eq!(std::fs::read_to_string(t.root.join("app/rel.txt")).unwrap(), "A");
        do_copy(&c, &s(&["a.txt"]), ".", &[], None).unwrap();
        assert!(t.root.join("app/a.txt").exists());
        // existing directory without trailing slash
        do_copy(&c, &s(&["a.txt"]), "/opt/dir", &[], None).unwrap();
        assert!(t.root.join("opt/dir/a.txt").exists());
    }

    #[test]
    fn directory_contents_and_globs() {
        let t = setup();
        put(&t.ctx_dir, "src/x.rs", "x");
        put(&t.ctx_dir, "src/deep/y.rs", "y");
        put(&t.ctx_dir, "f1.txt", "1");
        put(&t.ctx_dir, "f2.txt", "2");
        put(&t.ctx_dir, "other.md", "m");
        let c = cx(&t, None, "/", false);
        do_copy(&c, &s(&["src"]), "/dst", &[], None).unwrap();
        assert!(t.root.join("dst/x.rs").exists() && t.root.join("dst/deep/y.rs").exists());
        do_copy(&c, &s(&["f*.txt"]), "/globbed/", &[], None).unwrap();
        assert!(t.root.join("globbed/f1.txt").exists() && t.root.join("globbed/f2.txt").exists());
        assert!(!t.root.join("globbed/other.md").exists());
        // glob in a directory component
        do_copy(&c, &s(&["sr?/*.rs"]), "/g2/", &[], None).unwrap();
        assert!(t.root.join("g2/x.rs").exists());
        // multiple sources need a directory destination
        assert!(do_copy(&c, &s(&["f1.txt", "f2.txt"]), "/nodir", &[], None).is_err());
        assert!(do_copy(&c, &s(&["f*.txt"]), "/nodir", &[], None).is_err());
        assert!(do_copy(&c, &s(&["missing"]), "/x", &[], None).is_err());
        assert!(do_copy(&c, &s(&["nomatch*"]), "/x/", &[], None).is_err());
    }

    #[test]
    fn dockerignore_applies() {
        let t = setup();
        put(&t.ctx_dir, "keep.txt", "k");
        put(&t.ctx_dir, "secret.env", "s");
        put(&t.ctx_dir, "d/inner.log", "l");
        put(&t.ctx_dir, "d/inner.txt", "t");
        let ig = DockerIgnore::parse("*.env\n**/*.log\n");
        let c = cx(&t, Some(&ig), "/", false);
        do_copy(&c, &s(&["."]), "/out", &[], None).unwrap();
        assert!(t.root.join("out/keep.txt").exists());
        assert!(!t.root.join("out/secret.env").exists());
        assert!(!t.root.join("out/d/inner.log").exists());
        assert!(t.root.join("out/d/inner.txt").exists());
        assert!(do_copy(&c, &s(&["secret.env"]), "/x", &[], None).is_err());
        // digest ignores excluded files
        let d1 = source_digest(&c, &s(&["."]), &[]).unwrap();
        put(&t.ctx_dir, "secret.env", "changed");
        assert_eq!(d1, source_digest(&c, &s(&["."]), &[]).unwrap());
        put(&t.ctx_dir, "keep.txt", "changed");
        assert_ne!(d1, source_digest(&c, &s(&["."]), &[]).unwrap());
    }

    #[test]
    fn chmod_symlinks_and_heredoc() {
        let t = setup();
        put(&t.ctx_dir, "bin/run.sh", "#!/bin/sh");
        std::os::unix::fs::symlink("run.sh", t.ctx_dir.join("bin/alias")).unwrap();
        let c = cx(&t, None, "/", false);
        let touched = do_copy(&c, &s(&["bin"]), "/usr/bin2", &[], Some(0o755)).unwrap();
        let md = std::fs::metadata(t.root.join("usr/bin2/run.sh")).unwrap();
        assert_eq!(md.mode() & 0o777, 0o755);
        assert_eq!(std::fs::read_link(t.root.join("usr/bin2/alias")).unwrap(), PathBuf::from("run.sh"));
        assert!(touched.contains(&t.root.join("usr/bin2/run.sh")));
        assert!(touched.contains(&t.root.join("usr")));

        let hd = vec![("EOF".to_string(), "hello\n".to_string())];
        do_copy(&c, &s(&["<<EOF"]), "/etc/greeting", &hd, None).unwrap();
        assert_eq!(std::fs::read_to_string(t.root.join("etc/greeting")).unwrap(), "hello\n");
        do_copy(&c, &s(&["<<'EOF'"]), "/etc/d/", &hd, None).unwrap();
        assert!(t.root.join("etc/d/EOF").exists());
    }

    #[test]
    fn dest_symlink_stays_inside_root() {
        let t = setup();
        put(&t.ctx_dir, "f", "F");
        std::fs::create_dir_all(t.root.join("real")).unwrap();
        std::os::unix::fs::symlink("/real", t.root.join("link")).unwrap();
        let c = cx(&t, None, "/", false);
        do_copy(&c, &s(&["f"]), "/link/", &[], None).unwrap();
        assert!(t.root.join("real/f").exists());
    }

    #[test]
    fn add_extracts_tar_and_rejects_url() {
        let t = setup();
        let tar_path = t.ctx_dir.join("a.tar");
        {
            let f = std::fs::File::create(&tar_path).unwrap();
            let mut b = tar::Builder::new(f);
            let data = b"inside";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            b.append_data(&mut h, "pkg/file.txt", &data[..]).unwrap();
            b.finish().unwrap();
        }
        put(&t.ctx_dir, "plain.txt", "p");
        let c = cx(&t, None, "/", true);
        do_copy(&c, &s(&["a.tar"]), "/unpacked/", &[], None).unwrap();
        assert_eq!(std::fs::read_to_string(t.root.join("unpacked/pkg/file.txt")).unwrap(), "inside");
        do_copy(&c, &s(&["plain.txt"]), "/unpacked/", &[], None).unwrap();
        assert!(t.root.join("unpacked/plain.txt").exists());
        // COPY does not extract
        let c2 = cx(&t, None, "/", false);
        do_copy(&c2, &s(&["a.tar"]), "/raw/", &[], None).unwrap();
        assert!(t.root.join("raw/a.tar").exists());
        assert!(matches!(do_copy(&c, &s(&["https://example.com/x"]), "/y", &[], None), Err(Error::Unsupported(_))));
    }

    #[test]
    fn digest_is_content_sensitive() {
        let t = setup();
        put(&t.ctx_dir, "a", "1");
        let c = cx(&t, None, "/", false);
        let d1 = source_digest(&c, &s(&["a"]), &[]).unwrap();
        assert_eq!(d1, source_digest(&c, &s(&["a"]), &[]).unwrap());
        put(&t.ctx_dir, "a", "2");
        assert_ne!(d1, source_digest(&c, &s(&["a"]), &[]).unwrap());
        let h1 = source_digest(&c, &s(&["<<X"]), &[("X".into(), "a".into())]).unwrap();
        let h2 = source_digest(&c, &s(&["<<X"]), &[("X".into(), "b".into())]).unwrap();
        assert_ne!(h1, h2);
    }
}
