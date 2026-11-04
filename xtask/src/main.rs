use std::fmt::{Display, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::{env, fs};

use unseen_test_support::{Sha256, hex, json};

const USAGE: &str = "usage: cargo xtask header --check | wasm | xcframework | dist [<dir>] \
                     | pin [--check] [<dir>] | corpus [--check] [<src>] | tables [--check]";

const BINARYEN: &str = "133";

const MACOS: &str = "26.0";

const APPLE_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

const CORPUS: [&str; 3] = ["README.md", "text.json", "files.json"];

const CORPUS_PIN: &str = "testdata/textcorpus.sha256";

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["header", "--check"] => header(),
        ["header"] => Err("include/unseen.h is written by hand; `cargo xtask header --check` checks it".to_owned()),
        ["wasm"] => wasm().map(drop),
        ["xcframework"] => xcframework().map(drop),
        ["dist"] => dist(None),
        ["dist", dir] => dist(Some(Path::new(dir))),
        ["pin"] => pin(false, None),
        ["pin", "--check"] => pin(true, None),
        ["pin", "--check", dir] => pin(true, Some(Path::new(dir))),
        ["pin", dir] => pin(false, Some(Path::new(dir))),
        ["corpus"] => corpus(false, None),
        ["corpus", "--check"] => corpus(true, None),
        ["corpus", "--check", src] => corpus(true, Some(Path::new(src))),
        ["corpus", src] => corpus(false, Some(Path::new(src))),
        ["tables"] => tables(false),
        ["tables", "--check"] => tables(true),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn header() -> Result {
    let root = root();
    let version = version()?;
    run(cargo().args(["build", "-p", "unseen-ffi", "--locked"]))?;
    let work = root.join("target/staging/header");
    fresh_dir(&work)?;
    let program = work.join("header");
    let ffi = root.join("crates/unseen-ffi");
    let mut cc = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()));
    cc.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg(format!("-DUNSEEN_EXPECTED_VERSION=\"{version}\""))
        .arg("-I")
        .arg(ffi.join("include"))
        .arg(ffi.join("tests/header.c"))
        .arg(root.join("target/debug/libunseen_ffi.a"))
        .arg("-o")
        .arg(&program);
    if cfg!(target_os = "linux") {
        // What the Rust standard library of a static library needs from the system on glibc.
        cc.args(["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc"]);
    }
    run(&mut cc)?;
    run(&mut Command::new(&program))
}

fn wasm() -> Result<PathBuf> {
    let root = root();
    let version = version()?;
    let pkg = root.join("packages/npm");
    let manifest = read_text(&pkg.join("package.json"))?;
    let npm = json::parse(&manifest).map_err(at(pkg.join("package.json").display()))?;
    if npm.get("version").as_str() != Some(version.as_str()) {
        return Err(format!("packages/npm/package.json has another version than Cargo.toml {version}"));
    }

    let opt = env::var_os("WASM_OPT").map_or_else(|| PathBuf::from("wasm-opt"), PathBuf::from);
    let found = capture(Command::new(&opt).arg("--version"))?;
    if found != format!("wasm-opt version {BINARYEN}") && !found.starts_with(&format!("wasm-opt version {BINARYEN} ")) {
        return Err(format!("{} is {found}, the package is built with binaryen {BINARYEN}", opt.display()));
    }

    run(cargo().args(["build", "-p", "unseen-ffi", "--release", "--locked", "--target", "wasm32-unknown-unknown"]))?;
    let out = pkg.join("dist");
    fresh_dir(&out)?;
    run(Command::new(&opt)
        .args(["-O3", "--strip-debug", "--strip-producers"])
        .arg(root.join("target/wasm32-unknown-unknown/release/unseen_ffi.wasm"))
        .arg("-o")
        .arg(out.join("unseen.wasm")))?;
    copy(&pkg.join("package.json"), &out.join("package.json"))?;
    copy(&root.join("LICENSE"), &out.join("LICENSE"))?;
    let module = read_text(&pkg.join("unseen.js"))?;
    let mark = "const VERSION = '@VERSION@';";
    if module.matches(mark).count() != 1 {
        return Err(format!("packages/npm/unseen.js must hold `{mark}` once"));
    }
    write(&out.join("unseen.js"), &module.replace(mark, &format!("const VERSION = '{version}';")))?;
    Ok(out)
}

fn xcframework() -> Result<PathBuf> {
    let root = root();
    header()?;
    let mut libraries = Vec::new();
    for target in APPLE_TARGETS {
        run(cargo().env("MACOSX_DEPLOYMENT_TARGET", MACOS).args([
            "build",
            "-p",
            "unseen-ffi",
            "--release",
            "--locked",
            "--target",
            target,
        ]))?;
        libraries.push(root.join(format!("target/{target}/release/libunseen_ffi.a")));
    }

    let work = root.join("target/staging/xcframework");
    fresh_dir(&work)?;
    // Headers in a directory named after the module, so that another XCFramework in the same app
    // does not collide with this one's module.modulemap.
    let headers = work.join("include/UnseenFFI");
    let include = root.join("crates/unseen-ffi/include");
    for file in ["unseen.h", "module.modulemap"] {
        copy(&include.join(file), &headers.join(file))?;
    }
    let library = work.join("libunseen_ffi.a");
    run(Command::new("lipo").arg("-create").args(&libraries).arg("-output").arg(&library))?;

    let framework = root.join("packages/swift/UnseenFFI.xcframework");
    if framework.exists() {
        fs::remove_dir_all(&framework).map_err(at(framework.display()))?;
    }
    run(Command::new("xcodebuild")
        .arg("-create-xcframework")
        .arg("-library")
        .arg(&library)
        .arg("-headers")
        .arg(work.join("include"))
        .arg("-output")
        .arg(&framework))?;
    Ok(framework)
}

fn dist(dir: Option<&Path>) -> Result {
    let root = root();
    let version = version()?;
    let dest = dir_or_default(dir, &version)?;
    if dest.exists() && fs::read_dir(&dest).map_err(at(dest.display()))?.next().is_some() {
        return Err(format!("{} is not empty", dest.display()));
    }

    let out = root.join("target/staging/release");
    fresh_dir(&out)?;
    let module = wasm()?;
    let framework = xcframework()?;

    let package = read_text(&module.join("package.json"))?;
    let base = json::parse(&package)
        .map_err(at("packages/npm/package.json"))?
        .get("name")
        .as_str()
        .ok_or("packages/npm/package.json has no name")?
        .to_owned();
    let npm = format!("unseen-v{version}-npm.tgz");
    let xcf = format!("unseen-v{version}-xcframework.zip");
    pack(&module, &out.join(&npm))?;
    zip(&framework, &out.join(&xcf))?;

    check_npm(&out.join(&npm))?;
    check_swift(&out.join(&xcf))?;

    let checksum = capture(Command::new("swift").args(["package", "compute-checksum"]).arg(out.join(&xcf)))?;
    let entries = [
        (npm.clone(), format!("\"product\": \"npm\", \"package\": {}", quote(&base))),
        (
            xcf.clone(),
            format!(
                "\"product\": \"xcframework\", \"module\": \"UnseenFFI\", \"targets\": [\"{}\", \"{}\"], \
                 \"macos\": \"{MACOS}\", \"header\": \"unseen.h\", \"swiftpm_checksum\": {}",
                APPLE_TARGETS[0],
                APPLE_TARGETS[1],
                quote(&checksum)
            ),
        ),
    ];

    let mut sums = String::new();
    let mut listed = Vec::new();
    for (name, extra) in &entries {
        let data = fs::read(out.join(name)).map_err(at(out.join(name).display()))?;
        let sha = hex(&Sha256::digest(&data));
        if *name == xcf && sha != checksum {
            return Err(format!("swift package compute-checksum {xcf} is {checksum}, not its SHA-256 {sha}"));
        }
        writeln!(sums, "{sha}  {name}").map_err(|e| e.to_string())?;
        listed.push(format!(
            "    {{ \"name\": {}, {extra}, \"size\": {}, \"sha256\": \"{sha}\" }}",
            quote(name),
            data.len()
        ));
    }
    write(&out.join("SHA256SUMS"), &sums)?;
    write(&out.join("release-manifest.json"), &manifest(&version, &listed)?)?;

    fs::create_dir_all(&dest).map_err(at(dest.display()))?;
    for (name, _) in &entries {
        copy(&out.join(name), &dest.join(name))?;
    }
    for name in ["SHA256SUMS", "release-manifest.json"] {
        copy(&out.join(name), &dest.join(name))?;
    }
    println!("{}", dest.display());
    Ok(())
}

fn manifest(version: &str, listed: &[String]) -> Result<String> {
    let root = root();
    let git = |args: &[&str]| capture(Command::new("git").args(args).current_dir(&root));
    let rustc =
        capture(Command::new(env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())).arg("-V").current_dir(&root))?;
    let xcode = capture(Command::new("xcodebuild").arg("-version"))?;
    let node = capture(Command::new("node").arg("--version"))?;
    let pnpm = capture(Command::new("pnpm").arg("--version"))?;
    let dirty = !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty();
    let manifest = format!(
        "{{\n  \"name\": \"unseen\",\n  \"version\": {},\n  \"commit\": {},\n  \"dirty\": {dirty},\n  \
         \"files\": [\n{}\n  ],\n  \"tools\": {{ \"rustc\": {}, \"binaryen\": \"{BINARYEN}\", \"xcode\": {}, \
         \"node\": {}, \"pnpm\": {} }}\n}}\n",
        quote(version),
        quote(&git(&["rev-parse", "HEAD"])?),
        listed.join(",\n"),
        quote(&rustc),
        quote(xcode.lines().next().unwrap_or_default()),
        quote(&node),
        quote(&pnpm),
    );
    json::parse(&manifest).map_err(|e| format!("release-manifest.json: {e}"))?;
    Ok(manifest)
}

// pnpm writes a lockfile where it runs, so it packs a copy.
fn pack(from: &Path, tgz: &Path) -> Result {
    let root = root();
    let staged = root.join("target/staging/npm");
    fresh_dir(&staged)?;
    copy(from, &staged)?;
    let pack = root.join("target/staging/pack");
    fresh_dir(&pack)?;
    run(Command::new("pnpm").args(["pack", "--pack-destination"]).arg(&pack).current_dir(&staged))?;
    let packed = fs::read_dir(&pack)
        .map_err(at(pack.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(at(pack.display()))?;
    let [archive] = packed.as_slice() else {
        return Err("pnpm pack did not produce one archive".to_owned());
    };
    copy(&archive.path(), tgz)
}

fn check_npm(tgz: &Path) -> Result {
    let root = root();
    let consumer = root.join("target/staging/consumer/npm");
    fresh_dir(&consumer)?;
    write(&consumer.join("package.json"), "{ \"private\": true, \"type\": \"module\" }\n")?;
    run(Command::new("pnpm").args(["add", "--offline", "--ignore-scripts"]).arg(tgz).current_dir(&consumer))?;
    run(Command::new("node").args(["--test", "packages/npm/test/"]).env("UNSEEN_PACKAGE", &consumer).current_dir(&root))
}

fn check_swift(zip: &Path) -> Result {
    let root = root();
    let consumer = root.join("target/staging/consumer/swift");
    fresh_dir(&consumer)?;
    // A path dependency takes its identity from the directory name, as a URL one from the
    // repository's.
    let package = consumer.join("unseen.rs");
    copy(&root.join("Package.swift"), &package.join("Package.swift"))?;
    for item in ["Sources", "Tests"] {
        copy(&root.join("packages/swift").join(item), &package.join("packages/swift").join(item))?;
    }
    run(Command::new("ditto").args(["-x", "-k"]).arg(zip).arg(package.join("packages/swift")))?;
    run(Command::new("swift")
        .arg("test")
        .arg("--package-path")
        .arg(&package)
        .env("UNSEEN_CORPUS", root.join("testdata/textcorpus")))?;

    let app = consumer.join("app");
    write(
        &app.join("Package.swift"),
        "// swift-tools-version: 6.2\n\
         import PackageDescription\n\n\
         let package = Package(\n\
         \x20   name: \"App\",\n\
         \x20   platforms: [.macOS(.v26)],\n\
         \x20   dependencies: [.package(path: \"../unseen.rs\")],\n\
         \x20   targets: [.executableTarget(name: \"App\", dependencies: [.product(name: \"Unseen\", package: \"unseen.rs\")])]\n\
         )\n",
    )?;
    write(
        &app.join("Sources/App/main.swift"),
        "import Unseen\n\nlet output = try Unseen.sanitize(\"a\\u{200B}b\")\nprecondition(output == \"ab\")\n",
    )?;
    run(Command::new("swift").arg("run").arg("--package-path").arg(&app))
}

const RELEASE_LINE: &str = "let release: (url: String, checksum: String)? = ";

fn pin(check: bool, dir: Option<&Path>) -> Result {
    let root = root();
    let version = version()?;
    let dir = dir_or_default(dir, &version)?;
    let text = read_text(&dir.join("release-manifest.json"))?;
    let manifest = json::parse(&text).map_err(at(dir.join("release-manifest.json").display()))?;
    if manifest.get("version").as_str() != Some(version.as_str()) {
        return Err(format!("{} is not a set of version {version}", dir.display()));
    }
    let file = manifest
        .get("files")
        .as_array()
        .and_then(|files| files.iter().find(|f| f.get("product").as_str() == Some("xcframework")))
        .ok_or_else(|| format!("{} lists no xcframework", dir.display()))?;
    let (Some(name), Some(sha), Some(checksum)) =
        (file.get("name").as_str(), file.get("sha256").as_str(), file.get("swiftpm_checksum").as_str())
    else {
        return Err(format!("{} has an incomplete xcframework entry", dir.display()));
    };
    if sha != checksum {
        return Err(format!("{name}: SwiftPM checksum {checksum} is not its SHA-256 {sha}"));
    }
    let repository = workspace_field("repository")?
        .ok_or("Cargo.toml has no workspace repository to name the release by".to_owned())?;
    let value = format!(
        "(url: {}, checksum: {})",
        quote(&format!("{repository}/releases/download/v{version}/{name}")),
        quote(checksum)
    );

    let path = root.join("Package.swift");
    let swift = read_text(&path)?;
    let lines: Vec<&str> = swift.lines().filter(|line| line.starts_with(RELEASE_LINE)).collect();
    let [current] = lines.as_slice() else {
        return Err(format!("Package.swift must hold one line starting `{RELEASE_LINE}`"));
    };
    let wanted = format!("{RELEASE_LINE}{value}");
    if check {
        if *current != wanted {
            return Err(format!("Package.swift does not name {name} of {}: run `cargo xtask pin`", dir.display()));
        }
        return Ok(());
    }
    write(&path, &swift.replacen(current, &wanted, 1))?;
    println!("Package.swift names {name} {checksum}");
    Ok(())
}

fn dir_or_default(dir: Option<&Path>, version: &str) -> Result<PathBuf> {
    match dir {
        Some(dir) => std::path::absolute(dir).map_err(at(dir.display())),
        None => Ok(root().join(format!("dist/v{version}"))),
    }
}

fn corpus(check: bool, src: Option<&Path>) -> Result {
    let root = root();
    let copy_dir = root.join("testdata/textcorpus");
    let pin_path = root.join(CORPUS_PIN);
    let source = src.map_or_else(|| root.join("../s3/testdata/textcorpus"), Path::to_path_buf);
    let git = |args: &[&str]| capture(Command::new("git").args(args).current_dir(&source));

    if !check {
        if !git(&["status", "--porcelain", "--", "."])?.is_empty() {
            return Err(format!("{} has uncommitted changes: pin a committed corpus", source.display()));
        }
        let commit = git(&["log", "-1", "--format=%H", "--", "."])?;
        let mut pin = format!("commit {commit}\n");
        for file in CORPUS {
            let data = fs::read(source.join(file)).map_err(at(source.join(file).display()))?;
            write_bytes(&copy_dir.join(file), &data)?;
            writeln!(pin, "{}  {file}", hex(&Sha256::digest(&data))).map_err(|e| e.to_string())?;
        }
        write(&pin_path, &pin)?;
        println!("testdata/textcorpus is the service's corpus at {commit}");
        return Ok(());
    }

    let pin = read_text(&pin_path)?;
    let mut commit = None;
    let mut pinned = Vec::new();
    for line in pin.lines() {
        if let Some(c) = line.strip_prefix("commit ") {
            commit = Some(c);
        } else if let Some((sha, file)) = line.split_once("  ") {
            pinned.push((file, sha));
        } else {
            return Err(format!("{CORPUS_PIN}: unexpected line {line:?}"));
        }
    }
    let commit = commit.ok_or_else(|| format!("{CORPUS_PIN} names no commit"))?;
    if pinned.iter().map(|(f, _)| *f).ne(CORPUS) {
        return Err(format!("{CORPUS_PIN} must list {}", CORPUS.join(", ")));
    }
    for (file, sha) in &pinned {
        let data = fs::read(copy_dir.join(file)).map_err(at(copy_dir.join(file).display()))?;
        if hex(&Sha256::digest(&data)) != *sha {
            return Err(format!("testdata/textcorpus/{file} differs from {CORPUS_PIN}: run `cargo xtask corpus`"));
        }
    }
    if src.is_none() && !source.is_dir() {
        println!("testdata/textcorpus matches {CORPUS_PIN}; no service checkout to compare with");
        return Ok(());
    }
    for (file, _) in &pinned {
        let original = Command::new("git")
            .args(["show", &format!("{commit}:./{file}")])
            .current_dir(&source)
            .stderr(Stdio::inherit())
            .output()
            .map_err(at("git show"))?;
        if !original.status.success() {
            return Err(format!("{} has no {file} at {commit}", source.display()));
        }
        if fs::read(copy_dir.join(file)).map_err(at(copy_dir.join(file).display()))? != original.stdout {
            return Err(format!("testdata/textcorpus/{file} differs from the service at {commit}"));
        }
    }
    println!("testdata/textcorpus is the service's corpus at {commit}");
    Ok(())
}

fn tables(check: bool) -> Result {
    let root = root();
    let dir = root.join("ucd");
    fs::create_dir_all(&dir).map_err(at(dir.display()))?;
    let urls = capture(cargo().args(["run", "-q", "-p", "unseen-tablegen", "--locked", "--", "--urls"]))?;
    for line in urls.lines() {
        let (name, url) = line.split_once(' ').ok_or_else(|| format!("unseen-tablegen --urls: {line:?}"))?;
        let path = dir.join(name);
        if !path.exists() {
            run(Command::new("curl")
                .args([
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--location",
                    "--proto",
                    "=https",
                    "--tlsv1.2",
                    "--output",
                ])
                .arg(&path)
                .arg(url))?;
        }
    }
    let mut command = cargo();
    command.args(["run", "-q", "-p", "unseen-tablegen", "--locked", "--", "--source-dir"]).arg(&dir);
    if check {
        command.arg("--check");
    }
    run(&mut command)
}

fn zip(item: &Path, to: &Path) -> Result {
    let parent = item.parent().ok_or_else(|| format!("{} has no parent", item.display()))?;
    let name = item.file_name().ok_or_else(|| format!("{} has no name", item.display()))?;
    let mut entries = Vec::new();
    normalize(item, Path::new(name), &mut entries)?;
    entries.sort();
    let mut list = String::new();
    for entry in &entries {
        let entry = entry.to_str().ok_or_else(|| format!("{} is not UTF-8", entry.display()))?;
        if entry.contains('\n') {
            return Err(format!("{entry:?} cannot be listed for zip"));
        }
        list.push_str(entry);
        list.push('\n');
    }
    if to.exists() {
        fs::remove_file(to).map_err(at(to.display()))?;
    }
    let mut command = Command::new("zip");
    // ZIP stores local time: one zone keeps the fixed time one value.
    command.args(["-X", "-D", "-q", "-@"]).arg(to).current_dir(parent).env("TZ", "UTC");
    eprintln!("+ {}", show(&command));
    let mut child = command.stdin(Stdio::piped()).spawn().map_err(at("zip"))?;
    let mut stdin = child.stdin.take().ok_or("zip has no stdin")?;
    std::io::Write::write_all(&mut stdin, list.as_bytes()).map_err(at("zip"))?;
    drop(stdin);
    let status = child.wait().map_err(at("zip"))?;
    if status.success() { Ok(()) } else { Err(format!("zip failed: {status}")) }
}

fn normalize(path: &Path, relative: &Path, entries: &mut Vec<PathBuf>) -> Result {
    use std::os::unix::fs::PermissionsExt as _;
    // 2000-01-01T00:00:00Z: ZIP times start in 1980.
    const TIME: u64 = 946_684_800;
    let meta = fs::symlink_metadata(path).map_err(at(path.display()))?;
    if meta.is_dir() {
        for entry in fs::read_dir(path).map_err(at(path.display()))? {
            let entry = entry.map_err(at(path.display()))?;
            normalize(&entry.path(), &relative.join(entry.file_name()), entries)?;
        }
        return Ok(());
    }
    if !meta.is_file() {
        return Err(format!("{} is neither a file nor a directory", path.display()));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).map_err(at(path.display()))?;
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(TIME)))
        .map_err(at(path.display()))?;
    entries.push(relative.to_path_buf());
    Ok(())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask sits in the workspace root").to_path_buf()
}

fn version() -> Result<String> {
    workspace_field("version")?.ok_or_else(|| "Cargo.toml has no workspace version".to_owned())
}

fn workspace_field(key: &str) -> Result<Option<String>> {
    let manifest = read_text(&root().join("Cargo.toml"))?;
    let section = manifest
        .split_once("[workspace.package]\n")
        .map(|(_, rest)| rest.split("\n[").next().unwrap_or_default())
        .ok_or("Cargo.toml has no [workspace.package]")?;
    Ok(section.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
        value.strip_prefix('"')?.strip_suffix('"').map(str::to_owned)
    }))
}

fn cargo() -> Command {
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root());
    command
}

fn run(command: &mut Command) -> Result {
    eprintln!("+ {}", show(command));
    let status = command.status().map_err(at(show(command)))?;
    if status.success() { Ok(()) } else { Err(format!("{} failed: {status}", show(command))) }
}

fn capture(command: &mut Command) -> Result<String> {
    let output = command.stderr(Stdio::inherit()).output().map_err(at(show(command)))?;
    if !output.status.success() {
        return Err(format!("{} failed: {}", show(command), output.status));
    }
    String::from_utf8(output.stdout).map(|s| s.trim().to_owned()).map_err(at(show(command)))
}

fn show(command: &Command) -> String {
    let mut words = vec![command.get_program().to_string_lossy()];
    words.extend(command.get_args().map(|arg| arg.to_string_lossy()));
    words.join(" ")
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn fresh_dir(path: &Path) -> Result {
    if path.exists() {
        fs::remove_dir_all(path).map_err(at(path.display()))?;
    }
    fs::create_dir_all(path).map_err(at(path.display()))
}

fn copy(from: &Path, to: &Path) -> Result {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(at(parent.display()))?;
    }
    if from.is_dir() {
        fs::create_dir_all(to).map_err(at(to.display()))?;
        for entry in fs::read_dir(from).map_err(at(from.display()))? {
            let entry = entry.map_err(at(from.display()))?;
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(drop).map_err(at(from.display()))
    }
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(at(path.display()))
}

fn write(path: &Path, text: &str) -> Result {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, data: &[u8]) -> Result {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(at(parent.display()))?;
    }
    fs::write(path, data).map_err(at(path.display()))
}

fn at<E: Display>(what: impl Display) -> impl FnOnce(E) -> String {
    let what = what.to_string();
    move |error| format!("{what}: {error}")
}
