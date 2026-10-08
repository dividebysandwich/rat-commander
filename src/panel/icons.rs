//! Nerd Font glyphs for listing entries.
//!
//! With *Options → Settings → Nerd Font symbols* enabled, the listing's leading
//! `ls -F` classify character (`/`, `*`, `@`, `!`, space) is replaced by a
//! per-type icon: a folder, a symlink, or a glyph chosen from the file's name or
//! extension. Everything here is from the [Nerd Fonts](https://nerdfonts.com)
//! private-use ranges, so it only renders with a patched font installed in the
//! terminal — hence the setting, which is off by default.
//!
//! The codepoints are the stable Font Awesome (`U+F0xx`–`U+F2xx`), Octicons,
//! Seti-UI, Devicons and Codicons blocks; they have not moved between Nerd Fonts
//! v2 and v3. Prefer a **Mono** font variant: its icons are drawn one cell wide,
//! which is the width the listing reserves for them.

use crate::vfs::{VfsEntry, VfsKind};

// -- Structure ---------------------------------------------------------------
const DIR: char = '\u{f07b}'; // fa-folder
const DIR_UP: char = '\u{f062}'; // fa-arrow_up (the ".." entry)
const LINK: char = '\u{f0c1}'; // fa-link
const LINK_BROKEN: char = '\u{f127}'; // fa-unlink
const EXEC: char = '\u{f013}'; // fa-gear (a runnable file)
const FILE: char = '\u{f016}'; // fa-file_o (anything unrecognized)
const DEVICE: char = '\u{f2db}'; // fa-microchip (device / fifo / socket)

// -- Categories --------------------------------------------------------------
const TEXT: char = '\u{f0f6}'; // fa-file_text_o
const MARKDOWN: char = '\u{e73e}'; // dev-markdown
const PDF: char = '\u{f1c1}'; // fa-file_pdf_o
const WORD: char = '\u{f1c2}'; // fa-file_word_o
const SHEET: char = '\u{f1c3}'; // fa-file_excel_o
const CSV: char = '\u{e64a}'; // seti-csv
const SLIDES: char = '\u{f1c4}'; // fa-file_powerpoint_o
const BOOK: char = '\u{f02d}'; // fa-book
const LOG: char = '\u{f4ed}'; // oct-log
const IMAGE: char = '\u{f1c5}'; // fa-file_picture_o
const AUDIO: char = '\u{f001}'; // fa-music
const VIDEO: char = '\u{f008}'; // fa-film
const ARCHIVE: char = '\u{f1c6}'; // fa-file_zipper
const DISK_IMAGE: char = '\u{f0a0}'; // fa-hdd_o
const FONT: char = '\u{f031}'; // fa-font
const KEY: char = '\u{f084}'; // fa-key
const DATABASE: char = '\u{f1c0}'; // fa-database
const BINARY: char = '\u{f471}'; // oct-file_binary
const CODE: char = '\u{f1c9}'; // fa-file_code_o
const CONFIG: char = '\u{e615}'; // seti-config

// -- Languages, tools and platforms ------------------------------------------
const RUST: char = '\u{e7a8}'; // dev-rust
const PYTHON: char = '\u{e73c}'; // dev-python
const JAVASCRIPT: char = '\u{e74e}'; // dev-javascript_alt
const TYPESCRIPT: char = '\u{e628}'; // seti-typescript
const REACT: char = '\u{e7ba}'; // dev-react
const HTML: char = '\u{e736}'; // dev-html5
const CSS: char = '\u{e749}'; // dev-css3
const JSON: char = '\u{e80b}'; // dev-json
const XML: char = '\u{e619}'; // seti-xml
const YAML: char = '\u{e6a8}'; // seti-yml
const TOML: char = '\u{e6b2}'; // custom-toml
const C: char = '\u{e61e}'; // custom-c
const CPP: char = '\u{e61d}'; // custom-cpp
const CSHARP: char = '\u{e648}'; // seti-c_sharp
const JAVA: char = '\u{e738}'; // dev-java
const KOTLIN: char = '\u{e634}'; // seti-kotlin
const SCALA: char = '\u{e737}'; // dev-scala
const GO: char = '\u{e627}'; // seti-go
const RUBY: char = '\u{e739}'; // dev-ruby
const PHP: char = '\u{e73d}'; // dev-php
const LUA: char = '\u{e620}'; // seti-lua
const SWIFT: char = '\u{e755}'; // dev-swift
const HASKELL: char = '\u{e777}'; // dev-haskell
const PERL: char = '\u{e769}'; // dev-perl
const ELIXIR: char = '\u{e62d}'; // seti-elixir
const CLOJURE: char = '\u{e768}'; // dev-clojure
const DART: char = '\u{e798}'; // dev-dart
const ZIG: char = '\u{e6a9}'; // seti-zig
const NIM: char = '\u{e677}'; // seti-nim
const R: char = '\u{e68a}'; // seti-r
const ASM: char = '\u{e6ab}'; // custom-asm
const VIM: char = '\u{e62b}'; // custom-vim
const SHELL: char = '\u{e691}'; // seti-shell
const POWERSHELL: char = '\u{ebc7}'; // cod-terminal_powershell
const GIT: char = '\u{e702}'; // dev-git
const DOCKER: char = '\u{e7b0}'; // dev-docker
const MAKE: char = '\u{e673}'; // seti-makefile
const CMAKE: char = '\u{e794}'; // dev-cmake
const NIX: char = '\u{e843}'; // dev-nixos
const LICENSE: char = '\u{e60a}'; // seti-license
const DEBIAN: char = '\u{e77d}'; // dev-debian
const REDHAT: char = '\u{e7bb}'; // dev-redhat
const ANDROID: char = '\u{e70e}'; // dev-android
const WINDOWS: char = '\u{e70f}'; // dev-windows
const APPLE: char = '\u{e711}'; // dev-apple

/// The Nerd Font glyph for `e`: its kind for directories and links, otherwise a
/// glyph picked from the file name (`Makefile`, `Cargo.toml`, …) or extension,
/// falling back to a gear for anything executable and a blank page otherwise.
pub fn icon(e: &VfsEntry) -> char {
    match e.kind {
        VfsKind::Dir => {
            if e.name == ".." {
                DIR_UP
            } else {
                DIR
            }
        }
        VfsKind::Symlink => {
            if e.symlink_broken {
                LINK_BROKEN
            } else {
                LINK
            }
        }
        VfsKind::File => file_icon(e),
        // Character/block devices, fifos and sockets.
        _ => DEVICE,
    }
}

/// The glyph for a regular file: a known name wins over a known extension (a
/// `Dockerfile` has none at all), then executables get the gear.
fn file_icon(e: &VfsEntry) -> char {
    by_name(&e.name.to_ascii_lowercase())
        .or_else(|| by_extension(&e.extension().to_ascii_lowercase()))
        .unwrap_or(if e.is_executable() { EXEC } else { FILE })
}

/// A glyph for a well-known *whole* file name (already lowercased), for the
/// files that carry no useful extension.
fn by_name(name: &str) -> Option<char> {
    Some(match name {
        "makefile" | "gnumakefile" | "makefile.am" | "makefile.in" | "meson.build" => MAKE,
        "cmakelists.txt" | "cmakecache.txt" => CMAKE,
        "dockerfile"
        | "containerfile"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yml"
        | "compose.yaml"
        | ".dockerignore" => DOCKER,
        "license" | "license.md" | "license.txt" | "licence" | "copying" | "copying.lesser" => {
            LICENSE
        }
        "readme" | "readme.md" | "readme.txt" | "changelog" | "changelog.md" | "news"
        | "authors" => BOOK,
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" | ".mailmap" => GIT,
        ".editorconfig" | ".env" | ".clang-format" => CONFIG,
        "cargo.toml" | "cargo.lock" | "rust-toolchain.toml" | "rustfmt.toml" => RUST,
        "package.json" | "package-lock.json" | "tsconfig.json" | ".npmrc" => JAVASCRIPT,
        ".bashrc" | ".bash_profile" | ".bash_logout" | ".zshrc" | ".zprofile" | ".profile"
        | ".inputrc" => SHELL,
        ".vimrc" | ".gvimrc" | ".nvimrc" => VIM,
        "flake.nix" | "flake.lock" | "shell.nix" | "default.nix" => NIX,
        "pyproject.toml" | "requirements.txt" | "pipfile" | "setup.py" | "setup.cfg" => PYTHON,
        "gemfile" | "gemfile.lock" | "rakefile" => RUBY,
        "go.mod" | "go.sum" => GO,
        "id_rsa" | "id_ed25519" | "authorized_keys" | "known_hosts" => KEY,
        _ => return None,
    })
}

/// A glyph for a lowercased extension (without the dot).
fn by_extension(ext: &str) -> Option<char> {
    Some(match ext {
        // -- Documents --
        "txt" | "text" | "nfo" | "tex" | "latex" | "bib" | "cls" | "sty" => TEXT,
        "md" | "markdown" | "mdown" | "mkd" | "rst" | "adoc" | "org" => MARKDOWN,
        "pdf" | "ps" | "eps" | "xps" => PDF,
        "doc" | "docx" | "odt" | "rtf" | "wpd" | "abw" | "pages" => WORD,
        "xls" | "xlsx" | "xlsm" | "ods" | "numbers" => SHEET,
        "csv" | "tsv" => CSV,
        "ppt" | "pptx" | "odp" => SLIDES,
        "epub" | "mobi" | "azw" | "azw3" | "djvu" | "fb2" | "chm" => BOOK,
        "log" | "logs" => LOG,

        // -- Media --
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "svgz" | "webp" | "tiff" | "tif"
        | "ico" | "icns" | "ppm" | "pgm" | "pbm" | "pnm" | "xpm" | "heic" | "heif" | "avif"
        | "jxl" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "raf" | "psd" | "xcf"
        | "ai" | "kra" => IMAGE,
        "wav" | "mp3" | "flac" | "ogg" | "oga" | "opus" | "aac" | "m4a" | "m4b" | "wma" | "mid"
        | "midi" | "aiff" | "aif" | "ape" | "wv" | "mka" | "au" | "ra" | "mod" | "xm" | "it"
        | "s3m" => AUDIO,
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "mpg" | "mpeg" | "m2v"
        | "3gp" | "ogv" | "mts" | "m2ts" | "vob" | "rm" | "rmvb" | "asf" | "divx" | "srt"
        | "sub" | "ass" | "vtt" => VIDEO,

        // -- Archives, disk images and packages --
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz"
        | "zst" | "tzst" | "lz" | "lz4" | "lzma" | "lzo" | "z" | "cab" | "arj" | "lha" | "lzh"
        | "ace" | "zoo" | "cpio" | "sit" | "sitx" | "war" | "ear" | "xar" | "shar" | "appimage"
        | "snap" | "flatpak" | "pak" | "pack" => ARCHIVE,
        "iso" | "img" | "dmg" | "vhd" | "vhdx" | "vmdk" | "qcow" | "qcow2" | "cue" | "nrg"
        | "mdf" | "toast" => DISK_IMAGE,
        "deb" | "ddeb" | "udeb" => DEBIAN,
        "rpm" | "srpm" => REDHAT,
        "apk" | "aab" | "dex" => ANDROID,
        "exe" | "msi" | "dll" | "bat" | "cmd" | "com" | "lnk" | "reg" | "cpl" | "sys" => WINDOWS,
        "pkg" | "app" | "ipa" | "plist" => APPLE,
        "jar" => JAVA,
        "whl" | "egg" => PYTHON,

        // -- Fonts, keys, data, build output --
        "ttf" | "otf" | "ttc" | "woff" | "woff2" | "eot" | "pfb" | "pfa" | "bdf" | "pcf"
        | "fnt" | "fon" => FONT,
        "pem" | "crt" | "cer" | "der" | "key" | "p12" | "pfx" | "gpg" | "pgp" | "asc" | "sig"
        | "kdbx" | "pub" | "keystore" => KEY,
        "db" | "sqlite" | "sqlite3" | "sql" | "mdb" | "accdb" | "dump" | "myd" | "frm" => DATABASE,
        "o" | "obj" | "so" | "a" | "lib" | "dylib" | "ko" | "elf" | "bin" | "class" | "pyc"
        | "pyo" | "wasm" | "rlib" | "rmeta" | "pdb" => BINARY,

        // -- Configuration --
        "toml" => TOML,
        "yaml" | "yml" => YAML,
        "json" | "jsonc" | "json5" | "geojson" => JSON,
        "xml" | "xsd" | "xsl" | "xslt" | "rss" | "atom" => XML,
        "ini" | "cfg" | "conf" | "config" | "properties" | "prefs" | "desktop" | "service"
        | "rc" | "env" | "lock" => CONFIG,

        // -- Code --
        "rs" => RUST,
        "py" | "pyw" | "pyi" | "pyx" | "ipynb" => PYTHON,
        "js" | "mjs" | "cjs" => JAVASCRIPT,
        "ts" | "cts" => TYPESCRIPT,
        "tsx" | "jsx" => REACT,
        "html" | "htm" | "xhtml" | "vue" | "svelte" | "hbs" | "ejs" | "jinja" | "j2" => HTML,
        "css" | "scss" | "sass" | "less" | "styl" => CSS,
        "c" | "h" => C,
        "cpp" | "cxx" | "cc" | "c++" | "hpp" | "hxx" | "hh" | "h++" | "ipp" | "tpp" => CPP,
        "cs" | "csx" | "csproj" | "sln" => CSHARP,
        "java" => JAVA,
        "kt" | "kts" => KOTLIN,
        "scala" | "sc" | "sbt" => SCALA,
        "go" => GO,
        "rb" | "erb" | "gemspec" => RUBY,
        "php" | "phtml" | "php3" | "php4" | "php5" => PHP,
        "lua" => LUA,
        "swift" => SWIFT,
        "hs" | "lhs" | "cabal" => HASKELL,
        "pl" | "pm" | "t" | "pod" => PERL,
        "ex" | "exs" | "eex" | "heex" => ELIXIR,
        "clj" | "cljs" | "cljc" | "edn" => CLOJURE,
        "dart" => DART,
        "zig" | "zon" => ZIG,
        "nim" | "nims" | "nimble" => NIM,
        "r" | "rmd" | "rdata" | "rds" => R,
        "nix" => NIX,
        "asm" | "s" | "nasm" => ASM,
        "vim" | "vimrc" => VIM,
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "tcsh" | "ash" | "dash" => SHELL,
        "ps1" | "psm1" | "psd1" => POWERSHELL,
        "patch" | "diff" | "rej" | "orig" => GIT,
        "m" | "mm" | "f" | "f90" | "f95" | "for" | "pas" | "pp" | "vb" | "vbs" | "groovy"
        | "gradle" | "awk" | "sed" | "tcl" | "el" | "lisp" | "scm" | "rkt" | "ml" | "mli"
        | "fs" | "fsx" | "erl" | "hrl" | "jl" | "cr" | "v" | "sv" | "sol" | "hx" | "coffee"
        | "cmake" | "mk" | "am" | "ac" | "spec" | "proto" | "thrift" | "graphql" | "gql" | "ll"
        | "bc" => CODE,

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(name: &str, kind: VfsKind, mode: u32) -> VfsEntry {
        VfsEntry {
            name: name.to_string(),
            kind,
            size: 0,
            mtime: Some(SystemTime::UNIX_EPOCH),
            atime: None,
            ctime: None,
            btime: None,
            inode: None,
            mode: Some(mode),
            uid: None,
            gid: None,
            symlink_target: None,
            symlink_broken: false,
            symlink_dir: false,
        }
    }

    fn file(name: &str) -> VfsEntry {
        entry(name, VfsKind::File, 0o644)
    }

    #[test]
    fn kinds_get_their_structural_glyph() {
        assert_eq!(icon(&entry("sub", VfsKind::Dir, 0o755)), DIR);
        assert_eq!(icon(&entry("..", VfsKind::Dir, 0o755)), DIR_UP);
        assert_eq!(icon(&entry("l", VfsKind::Symlink, 0o777)), LINK);
        let mut broken = entry("l", VfsKind::Symlink, 0o777);
        broken.symlink_broken = true;
        assert_eq!(icon(&broken), LINK_BROKEN);
        assert_eq!(icon(&entry("sda", VfsKind::Other, 0o660)), DEVICE);
    }

    #[test]
    fn extensions_map_to_their_language_or_category() {
        assert_eq!(icon(&file("main.rs")), RUST);
        assert_eq!(icon(&file("photo.JPG")), IMAGE, "case-insensitive");
        assert_eq!(icon(&file("clip.mkv")), VIDEO);
        assert_eq!(icon(&file("song.flac")), AUDIO);
        assert_eq!(icon(&file("manual.pdf")), PDF);
        assert_eq!(icon(&file("backup.tar.gz")), ARCHIVE);
        assert_eq!(icon(&file("notes.md")), MARKDOWN);
        assert_eq!(icon(&file("settings.toml")), TOML);
        assert_eq!(icon(&file("build.sh")), SHELL);
    }

    #[test]
    fn whole_names_win_over_extensions_and_the_executable_bit() {
        assert_eq!(icon(&file("Dockerfile")), DOCKER, "name-only files are known");
        assert_eq!(icon(&file("Makefile")), MAKE);
        assert_eq!(icon(&file("LICENSE")), LICENSE);
        assert_eq!(icon(&file(".gitignore")), GIT);
        // Cargo.toml is Rust's, not a generic TOML file — and an executable
        // script keeps its language glyph rather than the generic gear.
        assert_eq!(icon(&file("Cargo.toml")), RUST);
        assert_eq!(icon(&entry("configure.sh", VfsKind::File, 0o755)), SHELL);
    }

    #[test]
    fn unknown_files_fall_back_to_page_or_gear() {
        assert_eq!(icon(&file("notes.qqq")), FILE);
        assert_eq!(icon(&file("nodots")), FILE);
        assert_eq!(icon(&entry("a.out", VfsKind::File, 0o755)), EXEC, "runnable");
    }

    #[test]
    fn every_glyph_is_one_column_wide() {
        // The listing reserves a single cell for the marker, so a glyph that
        // measured two columns would shift every name on its row.
        use unicode_width::UnicodeWidthChar;
        let names = [
            "a.rs",
            "b.py",
            "c.png",
            "d.mkv",
            "e.zip",
            "f.pdf",
            "Makefile",
            "Dockerfile",
            "g.deb",
            "h.ttf",
            "i.sqlite",
            "j.o",
            "k.json",
            "l.ps1",
            "m.hs",
            "n.unknownext",
        ];
        let mut glyphs: Vec<char> = names.iter().map(|n| icon(&file(n))).collect();
        glyphs.extend([DIR, DIR_UP, LINK, LINK_BROKEN, EXEC, FILE, DEVICE]);
        for g in glyphs {
            assert_eq!(g.width(), Some(1), "{g:?} (U+{:04X}) must be one cell", g as u32);
        }
    }
}
