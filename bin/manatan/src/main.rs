mod io;

use std::{
    cmp::Ordering as CmpOrdering,
    collections::{BTreeMap, HashMap},
    env,
    fs::{self},
    io::Read,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

use anyhow::anyhow;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use clap::Parser;
use directories::{BaseDirs, ProjectDirs};
use eframe::{
    egui::{self},
    icon_data,
};
use manatan_server_public::{
    app::build_router_without_cors, build_state, config::Config as ManatanServerConfig,
};
use reqwest::{
    Client, Method, Url,
    header::{
        ACCEPT, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_ORIGIN,
        ACCESS_CONTROL_REQUEST_METHOD, AUTHORIZATION, CONTENT_TYPE, ORIGIN,
    },
};
use rust_embed::RustEmbed;
use self_update::update::ReleaseUpdate;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

#[cfg(feature = "embed-jre")]
use crate::io::extract_zip;
use crate::io::{extract_file, resolve_java};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_NAME: &str = "Manatan";
const REPO_OWNER: &str = "KolbyML";
const REPO_NAME: &str = "Manatan";
const LEGACY_REPO_NAME: &str = "Mangatan";
const LEGACY_DATA_DIR_NAME: &str = "mangatan";
const BIN_NAME: &str = "manatan";
const SUWAYOMI_HOST: &str = "127.0.0.1";
const SUWAYOMI_PORT: u16 = 4566;
const SUWAYOMI_HTTP_BASE_URL: &str = "http://127.0.0.1:4566";
const MAX_PAGES_RESPONSE_REWRITE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PAGE_IMAGE_CONTENT_TYPE_REWRITE_BYTES: usize = 64 * 1024 * 1024;
const MANGA_PAGE_CACHE_BUSTER: &str = "downloadfix2";

static MANGA_CHAPTER_LOCKS: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();

#[derive(Clone)]
struct DownloadedMangaFallbackState {
    db_path: PathBuf,
    downloads_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MangaPageEndpoint {
    Pages {
        manga_id: i64,
        chapter_index: i64,
    },
    Page {
        manga_id: i64,
        chapter_index: i64,
        page_index: usize,
    },
}

#[derive(Debug)]
struct DownloadedChapterLocation {
    chapter_dir: PathBuf,
    page_count: usize,
    cache_key: String,
}

#[derive(Debug, Deserialize)]
struct DownloadManifest {
    pages: Vec<String>,
    #[serde(default)]
    archive_file: Option<String>,
}

#[derive(Debug, Default)]
struct LocalMangaArchiveShimSummary {
    scanned_archives: usize,
    planned_archives: usize,
    planned_folders: usize,
    copied_archives: usize,
    refreshed_folders: usize,
    skipped_archives: usize,
}

#[derive(Debug)]
struct LocalMangaArchivePlan {
    source_path: PathBuf,
    target_dir: PathBuf,
    archive_name: String,
}

#[derive(Debug, Serialize)]
struct LocalMangaFileMap {
    chapters: BTreeMap<String, Vec<LocalMangaFileMapPage>>,
}

#[derive(Debug, Serialize)]
struct LocalMangaFileMapPage {
    page_index: usize,
    file_key: String,
    file_hash: String,
    file_size: u64,
    modified_at: u128,
}

static ICON_BYTES: &[u8] = include_bytes!("../resources/faviconlogo.png");
static JAR_BYTES: &[u8] = include_bytes!("../resources/Suwayomi-Server.jar");

#[cfg(feature = "embed-jre")]
static NATIVES_BYTES: &[u8] = include_bytes!("../resources/natives.zip");

#[derive(RustEmbed)]
#[folder = "resources/webui"]
struct FrontendAssets;

#[derive(Serialize)]
struct VersionResponse {
    version: String,
    variant: String,
}

#[derive(Clone, Debug, PartialEq)]
enum UpdateStatus {
    Idle,
    Checking,
    UpdateAvailable(String),
    UpToDate,
    Downloading,
    RestartRequired,
    Error(String),
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Runs the server without the GUI (Fixes Docker/Server deployments)
    #[arg(long, env = "MANATAN_HEADLESS")]
    headless: bool,

    /// Opens the web interface in the default browser after server start (Requires --headless)
    #[arg(long, requires = "headless")]
    open_page: bool,

    /// Sets the IP address to bind the server to
    #[arg(long, default_value = "0.0.0.0", env = "MANATAN_HOST")]
    host: Ipv4Addr,

    /// Sets the Port to bind the server to
    #[arg(long, default_value_t = 4568, env = "MANATAN_PORT")]
    port: u16,

    /// Path to the Manatan SQLite database
    #[arg(long, env = "MANATAN_DB_PATH")]
    db_path: Option<PathBuf>,

    /// Optional migration directory/file path
    #[arg(long, env = "MANATAN_MIGRATE_PATH")]
    migrate_path: Option<PathBuf>,

    /// Run Suwayomi in runtime-only mode
    #[arg(
        long,
        env = "MANATAN_RUNTIME_ONLY",
        default_value_t = false,
        action = clap::ArgAction::Set,
        value_parser = parse_boolish,
        value_name = "BOOL"
    )]
    runtime_only: bool,

    /// Runtime bridge URL used by Manatan server
    #[arg(long, env = "MANATAN_JAVA_URL")]
    java_url: Option<String>,

    /// Enable remote tracker search
    #[arg(
        long,
        env = "MANATAN_TRACKER_REMOTE_SEARCH",
        default_value_t = true,
        action = clap::ArgAction::Set,
        value_parser = parse_boolish,
        value_name = "BOOL"
    )]
    tracker_remote_search: bool,

    /// Tracker search cache TTL in seconds
    #[arg(
        long,
        env = "MANATAN_TRACKER_SEARCH_TTL_SECONDS",
        default_value_t = 3600
    )]
    tracker_search_ttl_seconds: i64,

    /// Downloads directory (absolute or relative to data dir)
    #[arg(long, env = "MANATAN_DOWNLOADS_PATH")]
    downloads_path: Option<PathBuf>,

    /// Aidoku index URL
    #[arg(long, env = "MANATAN_AIDOKU_INDEX")]
    aidoku_index_url: Option<String>,

    /// Enable Aidoku integration
    #[arg(
        long,
        env = "MANATAN_AIDOKU_ENABLED",
        default_value_t = true,
        action = clap::ArgAction::Set,
        value_parser = parse_boolish,
        value_name = "BOOL"
    )]
    aidoku_enabled: bool,

    /// Aidoku cache directory (absolute or relative to data dir)
    #[arg(long, env = "MANATAN_AIDOKU_CACHE")]
    aidoku_cache_path: Option<PathBuf>,

    /// Local manga directory (absolute or relative to data dir)
    #[arg(long, env = "MANATAN_LOCAL_MANGA_PATH")]
    local_manga_path: Option<PathBuf>,

    /// Local anime directory (absolute or relative to data dir)
    #[arg(long, env = "MANATAN_LOCAL_ANIME_PATH")]
    local_anime_path: Option<PathBuf>,

    /// Local novel directory (absolute or relative to data dir)
    #[arg(long, env = "MANATAN_LOCAL_LN_PATH")]
    local_novel_path: Option<PathBuf>,
}

fn parse_boolish(value: &str) -> Result<bool, String> {
    match value.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("Invalid boolean value: {value}")),
    }
}

fn resolve_path_option(
    option: Option<&PathBuf>,
    data_dir: &Path,
    default_relative: &str,
) -> String {
    match option {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => data_dir.join(path),
        None => data_dir.join(default_relative),
    }
    .to_string_lossy()
    .to_string()
}

fn run_local_manga_archive_shim(
    local_manga_dir: &Path,
) -> anyhow::Result<LocalMangaArchiveShimSummary> {
    let mut summary = LocalMangaArchiveShimSummary::default();
    if !local_manga_dir.is_dir() {
        return Ok(summary);
    }

    let mut plans_by_target: BTreeMap<PathBuf, Vec<LocalMangaArchivePlan>> = BTreeMap::new();
    for entry in fs::read_dir(local_manga_dir)? {
        let entry = entry?;
        let source_path = entry.path();
        if !source_path.is_file() || !is_local_manga_archive(&source_path) {
            continue;
        }
        summary.scanned_archives += 1;

        match create_local_manga_archive_plan(local_manga_dir, &source_path) {
            Ok(plan) => {
                summary.planned_archives += 1;
                debug!(
                    "Local manga archive shim planned {} -> {}",
                    source_path.display(),
                    plan.target_dir.display()
                );
                plans_by_target
                    .entry(plan.target_dir.clone())
                    .or_default()
                    .push(plan);
            }
            Err(err) => {
                summary.skipped_archives += 1;
                warn!(
                    "Skipping local manga archive {}: {err}",
                    source_path.display()
                );
            }
        }
    }
    summary.planned_folders = plans_by_target.len();

    for (target_dir, mut plans) in plans_by_target {
        plans.sort_by(|left, right| natural_cmp(&left.archive_name, &right.archive_name));
        let folder_needs_refresh = plans.iter().any(|plan| {
            let target_archive = target_dir.join(&plan.archive_name);
            !local_manga_regular_file_exists(&target_archive)
                || !files_have_same_contents(&plan.source_path, &target_archive).unwrap_or(false)
                || local_manga_filemap_needs_refresh(
                    &target_dir.join(".manatan-local-filemap.json"),
                )
                || find_cover_file(&target_dir).is_none()
        });

        if !folder_needs_refresh {
            debug!(
                "Local manga archive shim found existing converted folder {}",
                target_dir.display()
            );
            summary.skipped_archives += plans.len();
            continue;
        }

        let group_result = (|| -> anyhow::Result<usize> {
            fs::create_dir_all(&target_dir)?;
            let mut copied_archives = 0;
            for plan in &plans {
                let target_archive = target_dir.join(&plan.archive_name);
                if !local_manga_regular_file_exists(&target_archive)
                    || !files_have_same_contents(&plan.source_path, &target_archive)
                        .unwrap_or(false)
                {
                    fs::copy(
                        local_manga_fs_path(&plan.source_path),
                        local_manga_fs_path(&target_archive),
                    )?;
                    copied_archives += 1;
                }
            }

            refresh_local_manga_folder(&target_dir)?;
            Ok(copied_archives)
        })();

        match group_result {
            Ok(copied_archives) => {
                summary.copied_archives += copied_archives;
                summary.refreshed_folders += 1;
            }
            Err(err) => {
                summary.skipped_archives += plans.len();
                warn!(
                    "Skipping local manga folder {}: {err}",
                    target_dir.display()
                );
            }
        }
    }

    Ok(summary)
}

fn create_local_manga_archive_plan(
    local_manga_dir: &Path,
    source_path: &Path,
) -> anyhow::Result<LocalMangaArchivePlan> {
    let archive_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("archive filename is not valid UTF-8"))?
        .to_string();
    let archive_stem = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(&archive_name);

    let file = fs::File::open(local_manga_fs_path(source_path))?;
    let mut archive = zip::ZipArchive::new(file)?;
    let image_entries = list_archive_image_entries(&mut archive)?;
    if image_entries.is_empty() {
        return Err(anyhow!("archive has no supported image entries"));
    }

    let metadata_title = read_archive_entry_by_basename(&mut archive, "meta.json")
        .ok()
        .flatten()
        .and_then(|bytes| local_manga_title_from_meta_json(&bytes));
    let comic_title = read_archive_entry_by_basename(&mut archive, "ComicInfo.xml")
        .ok()
        .flatten()
        .and_then(|bytes| local_manga_title_from_comic_info(&bytes));
    let fallback_title = title_from_archive_stem(archive_stem);
    let title = metadata_title
        .or(comic_title)
        .unwrap_or(fallback_title)
        .trim()
        .to_string();
    let folder_name = sanitize_local_manga_folder_name(&title).unwrap_or_else(|| {
        sanitize_local_manga_folder_name(archive_stem).unwrap_or_else(|| "Manga".to_string())
    });

    Ok(LocalMangaArchivePlan {
        source_path: source_path.to_path_buf(),
        target_dir: local_manga_dir.join(folder_name),
        archive_name,
    })
}

fn refresh_local_manga_folder(target_dir: &Path) -> anyhow::Result<()> {
    let folder_name = target_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("local manga folder name is not valid UTF-8"))?
        .to_string();
    let mut archives = fs::read_dir(target_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| local_manga_regular_file_exists(path) && is_local_manga_archive(path))
        .collect::<Vec<_>>();
    archives.sort_by(|left, right| {
        natural_cmp(
            &path_file_name_for_sort(left),
            &path_file_name_for_sort(right),
        )
    });

    let mut chapters = BTreeMap::new();
    let mut cover_written = false;
    let mut comic_info_written = false;

    for archive_path in archives {
        let archive_name = archive_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("archive filename is not valid UTF-8"))?
            .to_string();
        let archive_metadata = fs::metadata(local_manga_fs_path(&archive_path))?;
        let archive_file_size = archive_metadata.len();
        let archive_modified_at = metadata_modified_at_nanos(&archive_metadata);
        let archive_key_path = archive_path.clone();

        let file = fs::File::open(local_manga_fs_path(&archive_path))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let image_entries = list_archive_image_entries(&mut archive)?;
        if image_entries.is_empty() {
            continue;
        }

        if !cover_written {
            let cover_entry = &image_entries[0];
            let cover_ext = extension_for_archive_entry(cover_entry)
                .map(|extension| extension.to_ascii_lowercase())
                .unwrap_or_else(|| "jpg".to_string());
            let cover_path = target_dir.join(format!("cover.{cover_ext}"));
            let cover_bytes = read_archive_entry_bytes(&mut archive, cover_entry)?;
            write_if_changed(&cover_path, &cover_bytes)?;
            cover_written = true;
        }

        if !comic_info_written
            && let Some(bytes) = read_archive_entry_by_basename(&mut archive, "ComicInfo.xml")?
        {
            write_if_changed(&target_dir.join("ComicInfo.xml"), &bytes)?;
            comic_info_written = true;
        }

        let mut pages = Vec::with_capacity(image_entries.len());
        for (page_index, image_entry) in image_entries.iter().enumerate() {
            let bytes = read_archive_entry_bytes(&mut archive, image_entry)?;
            pages.push(LocalMangaFileMapPage {
                page_index,
                file_key: format!("zip:{}::{image_entry}", archive_key_path.display()),
                file_hash: fingerprint_bytes_64(&bytes),
                file_size: archive_file_size,
                modified_at: archive_modified_at,
            });
        }

        chapters.insert(format!("{folder_name}/{archive_name}"), pages);
    }

    if chapters.is_empty() {
        return Ok(());
    }

    let file_map = LocalMangaFileMap { chapters };
    let bytes = serde_json::to_vec_pretty(&file_map)?;
    write_if_changed(&target_dir.join(".manatan-local-filemap.json"), &bytes)?;
    Ok(())
}

fn is_local_manga_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension.to_ascii_lowercase().as_str(), "zip" | "cbz"))
        .unwrap_or(false)
}

fn local_manga_fs_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if value.starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        if let Some(unc_path) = value.strip_prefix(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{unc_path}"));
        }
        if path.is_absolute() {
            return PathBuf::from(format!(r"\\?\{value}"));
        }
    }

    path.to_path_buf()
}

fn local_manga_regular_file_exists(path: &Path) -> bool {
    fs::metadata(local_manga_fs_path(path))
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn local_manga_filemap_needs_refresh(path: &Path) -> bool {
    if !local_manga_regular_file_exists(path) {
        return true;
    }

    fs::read_to_string(local_manga_fs_path(path))
        .map(|text| text.contains(r#"zip:\\\\?\\"#))
        .unwrap_or(true)
}

fn list_archive_image_entries(
    archive: &mut zip::ZipArchive<fs::File>,
) -> anyhow::Result<Vec<String>> {
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        if entry.is_dir()
            || !is_supported_local_manga_image(&name)
            || is_ignored_archive_entry(&name)
        {
            continue;
        }
        entries.push(name);
    }
    entries.sort_by(|left, right| natural_cmp(left, right));
    Ok(entries)
}

fn read_archive_entry_by_basename(
    archive: &mut zip::ZipArchive<fs::File>,
    basename: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let mut matched_name = None;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if archive_entry_basename(&name).eq_ignore_ascii_case(basename) {
            matched_name = Some(name);
            break;
        }
    }

    matched_name
        .map(|name| read_archive_entry_bytes(archive, &name))
        .transpose()
}

fn read_archive_entry_bytes(
    archive: &mut zip::ZipArchive<fs::File>,
    entry_name: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut entry = archive.by_name(entry_name)?;
    let mut bytes = Vec::with_capacity(entry.size().try_into().unwrap_or(0));
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn is_supported_local_manga_image(name: &str) -> bool {
    extension_for_archive_entry(name)
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "avif" | "jxl"
            )
        })
        .unwrap_or(false)
}

fn is_ignored_archive_entry(name: &str) -> bool {
    name.split(['/', '\\']).any(|component| {
        component.eq_ignore_ascii_case("__MACOSX")
            || component.eq_ignore_ascii_case(".DS_Store")
            || component.eq_ignore_ascii_case("Thumbs.db")
    })
}

fn extension_for_archive_entry(name: &str) -> Option<&str> {
    archive_entry_basename(name)
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
}

fn archive_entry_basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn local_manga_title_from_meta_json(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let title = value.get("title")?;
    ["japanese", "english", "pretty", "romaji"]
        .iter()
        .filter_map(|key| title.get(*key)?.as_str())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn local_manga_title_from_comic_info(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let series = extract_xml_tag_text(text, "Series")
        .filter(|value| !value.eq_ignore_ascii_case("original"));
    series
        .or_else(|| extract_xml_tag_text(text, "Title"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn extract_xml_tag_text(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(unescape_minimal_xml(&text[start..end]))
}

fn unescape_minimal_xml(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn title_from_archive_stem(stem: &str) -> String {
    let (mut title, chapter_number) = strip_chapter_prefix(stem);
    if let Some(chapter_number) = chapter_number {
        title = strip_matching_trailing_number(&title, &chapter_number);
    }
    title.trim().to_string()
}

fn strip_chapter_prefix(stem: &str) -> (String, Option<String>) {
    let trimmed = stem.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("chapter ") {
        return (trimmed.to_string(), None);
    }

    let Some(separator_index) = trimmed.find(" - ") else {
        return (trimmed.to_string(), None);
    };
    let chapter_number = trimmed["chapter ".len()..separator_index].trim();
    if chapter_number.is_empty()
        || !chapter_number
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | ' '))
    {
        return (trimmed.to_string(), None);
    }

    (
        trimmed[separator_index + " - ".len()..].trim().to_string(),
        Some(chapter_number.to_string()),
    )
}

fn strip_matching_trailing_number(title: &str, chapter_number: &str) -> String {
    let Some((prefix, trailing)) = title.rsplit_once(' ') else {
        return title.to_string();
    };
    if normalize_chapter_number(trailing) == normalize_chapter_number(chapter_number) {
        prefix.trim().to_string()
    } else {
        title.to_string()
    }
}

fn normalize_chapter_number(value: &str) -> String {
    let normalized = value.trim().trim_start_matches('0');
    if normalized.is_empty() {
        "0".to_string()
    } else {
        normalized.to_string()
    }
}

fn sanitize_local_manga_folder_name(value: &str) -> Option<String> {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .trim()
        .to_string();

    (!sanitized.is_empty()).then_some(sanitized)
}

fn find_cover_file(target_dir: &Path) -> Option<PathBuf> {
    fs::read_dir(target_dir)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            (path.is_file() && file_name.to_ascii_lowercase().starts_with("cover.")).then_some(path)
        })
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let fs_path = local_manga_fs_path(path);
    if local_manga_regular_file_exists(path)
        && let Ok(existing) = fs::read(&fs_path)
        && existing == bytes
    {
        return Ok(());
    }
    fs::write(fs_path, bytes)
}

fn files_have_same_contents(left: &Path, right: &Path) -> std::io::Result<bool> {
    let left_fs_path = local_manga_fs_path(left);
    let right_fs_path = local_manga_fs_path(right);
    let left_metadata = fs::metadata(&left_fs_path)?;
    let right_metadata = fs::metadata(&right_fs_path)?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let mut left_file = fs::File::open(left_fs_path)?;
    let mut right_file = fs::File::open(right_fs_path)?;
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];

    loop {
        let left_read = left_file.read(&mut left_buffer)?;
        let right_read = right_file.read(&mut right_buffer)?;
        if left_read != right_read {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
        if left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
    }
}

fn metadata_modified_at_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_else(|| Duration::from_secs(0))
        .as_nanos()
}

fn fingerprint_bytes_64(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn path_file_name_for_sort(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn natural_cmp(left: &str, right: &str) -> CmpOrdering {
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left_chars.len() && right_index < right_chars.len() {
        let left_char = left_chars[left_index];
        let right_char = right_chars[right_index];

        if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
            let left_start = left_index;
            let right_start = right_index;
            while left_index < left_chars.len() && left_chars[left_index].is_ascii_digit() {
                left_index += 1;
            }
            while right_index < right_chars.len() && right_chars[right_index].is_ascii_digit() {
                right_index += 1;
            }
            let left_digits = left_chars[left_start..left_index]
                .iter()
                .collect::<String>();
            let right_digits = right_chars[right_start..right_index]
                .iter()
                .collect::<String>();
            let left_trimmed = left_digits.trim_start_matches('0');
            let right_trimmed = right_digits.trim_start_matches('0');
            let left_number = if left_trimmed.is_empty() {
                "0"
            } else {
                left_trimmed
            };
            let right_number = if right_trimmed.is_empty() {
                "0"
            } else {
                right_trimmed
            };

            match left_number.len().cmp(&right_number.len()) {
                CmpOrdering::Equal => match left_number.cmp(right_number) {
                    CmpOrdering::Equal => match left_digits.len().cmp(&right_digits.len()) {
                        CmpOrdering::Equal => {}
                        ordering => return ordering,
                    },
                    ordering => return ordering,
                },
                ordering => return ordering,
            }
            continue;
        }

        match left_char
            .to_ascii_lowercase()
            .cmp(&right_char.to_ascii_lowercase())
        {
            CmpOrdering::Equal => {
                left_index += 1;
                right_index += 1;
            }
            ordering => return ordering,
        }
    }

    left_chars.len().cmp(&right_chars.len())
}

fn resolve_data_dir() -> PathBuf {
    let new_proj_dirs =
        ProjectDirs::from("", "", APP_NAME).expect("Could not determine home directory");
    let legacy_proj_dirs = ProjectDirs::from("", "", LEGACY_DATA_DIR_NAME)
        .expect("Could not determine home directory");

    let new_dir = new_proj_dirs.data_dir().to_path_buf();
    let legacy_dir = legacy_proj_dirs.data_dir().to_path_buf();

    if new_dir == legacy_dir {
        return new_dir;
    }

    if new_dir.exists() {
        if legacy_dir.exists() && new_dir.is_dir() && is_dir_empty(&new_dir) {
            if let Err(err) = fs::remove_dir_all(&new_dir) {
                warn!(
                    "Failed to remove empty data dir {}: {err}",
                    new_dir.display()
                );
                return new_dir;
            }
            return migrate_legacy_data_dir(&legacy_dir, &new_dir);
        }

        if legacy_dir.exists() {
            warn!(
                "Legacy data dir still exists at {}. Using new data dir at {}.",
                legacy_dir.display(),
                new_dir.display()
            );
        }

        return new_dir;
    }

    if legacy_dir.exists() {
        return migrate_legacy_data_dir(&legacy_dir, &new_dir);
    }

    new_dir
}

fn migrate_legacy_data_dir(legacy_dir: &Path, new_dir: &Path) -> PathBuf {
    if let Some(parent) = new_dir.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        warn!(
            "Failed to create data dir parent {}: {err}",
            parent.display()
        );
        return legacy_dir.to_path_buf();
    }

    match fs::rename(legacy_dir, new_dir) {
        Ok(()) => {
            info!(
                "Migrated data dir from {} to {}",
                legacy_dir.display(),
                new_dir.display()
            );
            new_dir.to_path_buf()
        }
        Err(err) => {
            warn!(
                "Failed to move legacy data dir ({} -> {}): {err}. Falling back to copy.",
                legacy_dir.display(),
                new_dir.display()
            );
            match copy_dir_recursive(legacy_dir, new_dir) {
                Ok(()) => {
                    info!(
                        "Copied legacy data dir from {} to {}",
                        legacy_dir.display(),
                        new_dir.display()
                    );
                    new_dir.to_path_buf()
                }
                Err(copy_err) => {
                    warn!(
                        "Failed to copy legacy data dir ({} -> {}): {copy_err}",
                        legacy_dir.display(),
                        new_dir.display()
                    );
                    legacy_dir.to_path_buf()
                }
            }
        }
    }
}

fn migrate_suwayomi_extensions(data_dir: &Path) {
    let base_dirs = match BaseDirs::new() {
        Some(base_dirs) => base_dirs,
        None => return,
    };

    let legacy_extensions_dir = base_dirs
        .data_local_dir()
        .join("Tachidesk")
        .join("extensions");
    if !legacy_extensions_dir.exists() {
        return;
    }

    let new_extensions_dir = data_dir.join("extensions");
    if new_extensions_dir.exists() && !is_dir_empty(&new_extensions_dir) {
        info!(
            "Manatan extensions already present at {}. Skipping Suwayomi extension migration.",
            new_extensions_dir.display()
        );
        return;
    }

    if new_extensions_dir.exists()
        && is_dir_empty(&new_extensions_dir)
        && let Err(err) = fs::remove_dir_all(&new_extensions_dir)
    {
        warn!(
            "Failed to remove empty extensions dir {}: {err}",
            new_extensions_dir.display()
        );
    }

    if let Some(parent) = new_extensions_dir.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        warn!(
            "Failed to create extensions parent dir {}: {err}",
            parent.display()
        );
    }

    match fs::rename(&legacy_extensions_dir, &new_extensions_dir) {
        Ok(()) => {
            info!(
                "Moved Suwayomi extensions from {} to {}",
                legacy_extensions_dir.display(),
                new_extensions_dir.display()
            );
        }
        Err(err) => {
            warn!(
                "Failed to move Suwayomi extensions ({} -> {}): {err}. Falling back to copy.",
                legacy_extensions_dir.display(),
                new_extensions_dir.display()
            );
            match copy_dir_recursive(&legacy_extensions_dir, &new_extensions_dir) {
                Ok(()) => {
                    info!(
                        "Copied Suwayomi extensions from {} to {}",
                        legacy_extensions_dir.display(),
                        new_extensions_dir.display()
                    );
                    if let Err(remove_err) = fs::remove_dir_all(&legacy_extensions_dir) {
                        warn!(
                            "Failed to remove legacy extensions dir {}: {remove_err}",
                            legacy_extensions_dir.display()
                        );
                    }
                }
                Err(copy_err) => {
                    warn!(
                        "Failed to copy Suwayomi extensions ({} -> {}): {copy_err}",
                        legacy_extensions_dir.display(),
                        new_extensions_dir.display()
                    );
                }
            }
        }
    }
}

fn migrate_suwayomi_database(data_dir: &Path) {
    let base_dirs = match BaseDirs::new() {
        Some(base_dirs) => base_dirs,
        None => return,
    };

    let legacy_dir = base_dirs.data_local_dir().join("Tachidesk");
    let legacy_mv = legacy_dir.join("database.mv.db");
    let legacy_h2 = legacy_dir.join("database.h2.db");
    if !legacy_mv.exists() && !legacy_h2.exists() {
        return;
    }

    let new_mv = data_dir.join("database.mv.db");
    let new_h2 = data_dir.join("database.h2.db");
    if new_mv.exists() || new_h2.exists() {
        info!(
            "Manatan database already present at {}. Skipping Suwayomi database migration.",
            data_dir.display()
        );
        return;
    }

    if let Err(err) = fs::create_dir_all(data_dir) {
        warn!("Failed to create data dir {}: {err}", data_dir.display());
        return;
    }

    for (legacy, new_path) in [(legacy_mv, new_mv), (legacy_h2, new_h2)] {
        if !legacy.exists() {
            continue;
        }
        match fs::rename(&legacy, &new_path) {
            Ok(()) => {
                info!(
                    "Moved Suwayomi database file from {} to {}",
                    legacy.display(),
                    new_path.display()
                );
            }
            Err(err) => {
                warn!(
                    "Failed to move Suwayomi database file ({} -> {}): {err}. Falling back to copy.",
                    legacy.display(),
                    new_path.display()
                );
                match fs::copy(&legacy, &new_path) {
                    Ok(_) => {
                        info!(
                            "Copied Suwayomi database file from {} to {}",
                            legacy.display(),
                            new_path.display()
                        );
                        if let Err(remove_err) = fs::remove_file(&legacy) {
                            warn!(
                                "Failed to remove legacy database file {}: {remove_err}",
                                legacy.display()
                            );
                        }
                    }
                    Err(copy_err) => {
                        warn!(
                            "Failed to copy Suwayomi database file ({} -> {}): {copy_err}",
                            legacy.display(),
                            new_path.display()
                        );
                    }
                }
            }
        }
    }
}

fn migrate_suwayomi_settings(data_dir: &Path) {
    let base_dirs = match BaseDirs::new() {
        Some(base_dirs) => base_dirs,
        None => return,
    };

    let legacy_settings_dir = base_dirs
        .data_local_dir()
        .join("Tachidesk")
        .join("settings");
    if !legacy_settings_dir.exists() {
        return;
    }

    let new_settings_dir = data_dir.join("settings");
    if new_settings_dir.exists() && !is_dir_empty(&new_settings_dir) {
        if let Err(err) = copy_missing_settings_files(&legacy_settings_dir, &new_settings_dir) {
            warn!(
                "Failed to merge Suwayomi settings ({} -> {}): {err}",
                legacy_settings_dir.display(),
                new_settings_dir.display()
            );
        } else {
            info!(
                "Merged Suwayomi settings into {}",
                new_settings_dir.display()
            );
        }
        return;
    }

    if new_settings_dir.exists()
        && is_dir_empty(&new_settings_dir)
        && let Err(err) = fs::remove_dir_all(&new_settings_dir)
    {
        warn!(
            "Failed to remove empty settings dir {}: {err}",
            new_settings_dir.display()
        );
    }

    if let Some(parent) = new_settings_dir.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        warn!(
            "Failed to create settings parent dir {}: {err}",
            parent.display()
        );
    }

    match fs::rename(&legacy_settings_dir, &new_settings_dir) {
        Ok(()) => {
            info!(
                "Moved Suwayomi settings from {} to {}",
                legacy_settings_dir.display(),
                new_settings_dir.display()
            );
        }
        Err(err) => {
            warn!(
                "Failed to move Suwayomi settings ({} -> {}): {err}. Falling back to copy.",
                legacy_settings_dir.display(),
                new_settings_dir.display()
            );
            match copy_dir_recursive(&legacy_settings_dir, &new_settings_dir) {
                Ok(()) => {
                    info!(
                        "Copied Suwayomi settings from {} to {}",
                        legacy_settings_dir.display(),
                        new_settings_dir.display()
                    );
                    if let Err(remove_err) = fs::remove_dir_all(&legacy_settings_dir) {
                        warn!(
                            "Failed to remove legacy settings dir {}: {remove_err}",
                            legacy_settings_dir.display()
                        );
                    }
                }
                Err(copy_err) => {
                    warn!(
                        "Failed to copy Suwayomi settings ({} -> {}): {copy_err}",
                        legacy_settings_dir.display(),
                        new_settings_dir.display()
                    );
                }
            }
        }
    }
}

fn copy_missing_settings_files(source: &Path, dest: &Path) -> Result<(), std::io::Error> {
    if !source.exists() || !source.is_dir() {
        return Ok(());
    }
    if !dest.exists() {
        fs::create_dir_all(dest)?;
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dest.join(file_name);
        if source_path.is_dir() {
            copy_missing_settings_files(&source_path, &dest_path)?;
            continue;
        }
        if dest_path.exists() {
            continue;
        }
        fs::copy(&source_path, &dest_path)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

fn is_dir_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => false,
    }
}

fn main() -> eframe::Result<()> {
    if manatan_server_public::cef_app::try_handle_subprocess() {
        return Ok(());
    }

    let args = Cli::parse();

    let rust_log = env::var(EnvFilter::DEFAULT_ENV).unwrap_or_default();
    let env_filter = match rust_log.is_empty() {
        true => EnvFilter::builder().parse_lossy("info"),
        false => EnvFilter::builder().parse_lossy(rust_log),
    };
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let data_dir = resolve_data_dir();

    let server_data_dir = data_dir.clone();
    let gui_data_dir = data_dir.clone();

    let host = args.host;
    let port = args.port;

    if args.headless {
        info!("👻 Starting in Headless Mode (No GUI)...");

        let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

        rt.block_on(async {
            if args.open_page {
                tokio::spawn(async move { open_webpage_when_ready(host, port).await });
            }

            let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
            tokio::spawn(async move {
                wait_for_shutdown_signal().await;
                info!("🛑 Shutdown signal received, shutting down server...");
                let _ = shutdown_tx.send(()).await;
            });

            if let Err(err) = run_server(shutdown_rx, &server_data_dir, host, port, &args).await {
                error!("Server crashed: {err}");
            }
        });

        return Ok(());
    }

    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let (server_stopped_tx, server_stopped_rx) = std::sync::mpsc::channel::<()>();
    let shutdown_requested = Arc::new(AtomicBool::new(false));

    let thread_host = host;
    let thread_args = args.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        rt.block_on(async {
            let _guard = ServerGuard {
                tx: server_stopped_tx,
            };

            let h = thread_host;
            tokio::spawn(async move { open_webpage_when_ready(h, port).await });

            if let Err(err) = run_server(
                shutdown_rx,
                &server_data_dir,
                thread_host,
                port,
                &thread_args,
            )
            .await
            {
                error!("Server crashed: {err}");
            }
        });
    });

    let signal_shutdown_flag = Arc::clone(&shutdown_requested);
    let signal_shutdown_tx = shutdown_tx.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        rt.block_on(async move {
            wait_for_shutdown_signal().await;
            signal_shutdown_flag.store(true, Ordering::SeqCst);
            let _ = signal_shutdown_tx.send(()).await;
        });
    });

    let icon = icon_data::from_png_bytes(ICON_BYTES).expect("The icon data must be valid");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([320.0, 320.0])
            .with_icon(icon)
            .with_title(APP_NAME)
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };

    info!("🎨 Attempting to open GUI window...");
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |_cc| {
            Ok(Box::new(MyApp::new(
                shutdown_tx,
                server_stopped_rx,
                gui_data_dir,
                shutdown_requested,
                host,
                port,
            )))
        }),
    );

    if let Err(err) = &result {
        error!("❌ CRITICAL GUI ERROR: Failed to start eframe: {err}");
        std::thread::sleep(std::time::Duration::from_secs(5));
    } else {
        info!("👋 GUI exited normally.");
    }

    result
}

struct ServerGuard {
    tx: Sender<()>,
}
impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.tx.send(());
    }
}

struct MyApp {
    shutdown_tx: tokio::sync::mpsc::Sender<()>,
    server_stopped_rx: Receiver<()>,
    is_shutting_down: bool,
    data_dir: PathBuf,
    update_status: Arc<Mutex<UpdateStatus>>,
    shutdown_requested: Arc<AtomicBool>,
    host: Ipv4Addr,
    port: u16,
}

impl MyApp {
    fn new(
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
        server_stopped_rx: Receiver<()>,
        data_dir: PathBuf,
        shutdown_requested: Arc<AtomicBool>,
        host: Ipv4Addr,
        port: u16,
    ) -> Self {
        // Initialize status
        let update_status = Arc::new(Mutex::new(UpdateStatus::Idle));

        // Optional: Trigger a check immediately on startup
        let status_clone = update_status.clone();
        std::thread::spawn(move || {
            if !is_flatpak() {
                check_for_updates(status_clone);
            }
        });

        Self {
            shutdown_tx,
            server_stopped_rx,
            is_shutting_down: false,
            data_dir,
            update_status,
            shutdown_requested,
            host,
            port,
        }
    }

    fn begin_shutdown(&mut self, message: &str) {
        if !self.is_shutting_down {
            self.is_shutting_down = true;
            tracing::info!("{message} Signaling server to stop...");
            let _ = self.shutdown_tx.try_send(());
        }
    }

    fn trigger_update(&self) {
        let status_clone = self.update_status.clone();

        *status_clone.lock().expect("lock shouldn't panic") = UpdateStatus::Downloading;

        std::thread::spawn(move || match perform_update() {
            Ok(_) => {
                *status_clone.lock().expect("lock shouldn't panic") = UpdateStatus::RestartRequired
            }
            Err(e) => {
                *status_clone.lock().expect("lock shouldn't panic") =
                    UpdateStatus::Error(e.to_string())
            }
        });
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            self.begin_shutdown("🛑 Shutdown signal received.");
        }

        // Handle window close requests
        if ctx.input(|i| i.viewport().close_requested()) {
            self.begin_shutdown("❌ Close requested.");
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }

        if self.is_shutting_down {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.spinner();
                    ui.add_space(10.0);
                    ui.heading("Stopping Servers...");
                    ui.label("Cleaning up child processes...");
                });
            });

            if self.server_stopped_rx.try_recv().is_ok() {
                std::process::exit(0);
            }
            ctx.request_repaint();
            return;
        }

        // --- NORMAL UI ---

        // 1. Version Footer (Floating)
        egui::Area::new("version_watermark".into())
            .anchor(egui::Align2::LEFT_BOTTOM, [8.0, -8.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.weak(format!("v{APP_VERSION}"));
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // --- TOP HEADER: Title & Updates ---
            ui.horizontal(|ui| {
                ui.heading(APP_NAME);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_flatpak() {
                        ui.weak(format!("Flatpak: v{APP_VERSION}"));
                    } else {
                        let status = self
                            .update_status
                            .lock()
                            .expect("lock shouldn't panic")
                            .clone();
                        match status {
                            UpdateStatus::Idle | UpdateStatus::UpToDate => {
                                if ui.small_button("🔄 Check Updates").clicked() {
                                    let status_clone = self.update_status.clone();
                                    std::thread::spawn(move || check_for_updates(status_clone));
                                }
                            }
                            UpdateStatus::Checking => {
                                ui.spinner();
                            }
                            _ => {} // Handle active updates in the main body
                        }
                    }
                });
            });

            ui.separator();
            ui.add_space(10.0);

            // --- UPDATE NOTIFICATIONS AREA ---
            let status = self
                .update_status
                .lock()
                .expect("lock shouldn't panic")
                .clone();
            match status {
                UpdateStatus::UpdateAvailable(ver) => {
                    ui.group(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.colored_label(
                                egui::Color32::LIGHT_BLUE,
                                format!("✨ Update {ver} Available"),
                            );
                            ui.add_space(5.0);
                            if ui.button("⬇ Download & Install").clicked() {
                                self.trigger_update();
                            }
                        });
                    });
                    ui.add_space(10.0);
                }
                UpdateStatus::Downloading => {
                    ui.group(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.spinner();
                            ui.label("Downloading update...");
                        });
                    });
                    ui.add_space(10.0);
                }
                UpdateStatus::RestartRequired => {
                    ui.group(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.colored_label(egui::Color32::GREEN, "✔ Update Ready!");
                            ui.add_space(5.0);
                            if ui.button("🚀 Restart App").clicked() {
                                if let Ok(exe_path) = std::env::current_exe() {
                                    let mut exe_str = exe_path.to_string_lossy().to_string();
                                    if cfg!(target_os = "linux") && exe_str.ends_with(" (deleted)")
                                    {
                                        exe_str = exe_str.replace(" (deleted)", "");
                                    }
                                    let _ = std::process::Command::new(exe_str).spawn();
                                }
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                    ui.add_space(10.0);
                }
                UpdateStatus::Error(e) => {
                    ui.colored_label(egui::Color32::RED, "Update Failed");
                    ui.small(e.chars().take(40).collect::<String>());
                    if ui.button("Retry").clicked() {
                        *self.update_status.lock().expect("lock shouldn't panic") =
                            UpdateStatus::Idle;
                    }
                    ui.add_space(10.0);
                }
                _ => {}
            }

            // --- PRIMARY ACTION (THE "HERO" BUTTON) ---
            ui.vertical_centered(|ui| {
                ui.add_space(5.0);
                let btn_size = egui::vec2(ui.available_width() * 0.9, 45.0);
                let btn =
                    egui::Button::new(egui::RichText::new("🚀 OPEN WEB UI").size(18.0).strong())
                        .min_size(btn_size);

                if ui.add(btn).clicked() {
                    let host_target = if self.host == Ipv4Addr::new(0, 0, 0, 0) {
                        "localhost".to_string()
                    } else {
                        self.host.to_string()
                    };
                    let url = format!("http://{host_target}:{}", self.port);
                    let _ = open::that(url);
                }
            });

            ui.add_space(15.0);

            // --- SECONDARY ACTIONS (Community) ---
            ui.vertical_centered(|ui| {
                if ui.button("💬 Join Discord Community").clicked() {
                    let _ = open::that("https://discord.gg/tDAtpPN8KK");
                }
            });

            ui.add_space(15.0);
            ui.separator();

            // --- TERTIARY ACTIONS (Data Management) ---
            ui.add_space(5.0);
            ui.label("Data Management:");

            // Single action (keep spacing and avoid cramped bottom).
            if ui
                .add_sized(
                    [ui.available_width(), 30.0],
                    egui::Button::new("📂 Manatan Data"),
                )
                .clicked()
            {
                if !self.data_dir.exists() {
                    let _ = std::fs::create_dir_all(&self.data_dir);
                }
                let _ = open::that(&self.data_dir);
            }

            ui.add_space(12.0);
        });
    }
}

async fn run_server(
    mut shutdown_signal: tokio::sync::mpsc::Receiver<()>,
    data_dir: &PathBuf,
    host: Ipv4Addr,
    port: u16,
    cli: &Cli,
) -> Result<(), Box<anyhow::Error>> {
    info!("🚀 Initializing Manatan Launcher...");
    info!("📂 Data Directory: {}", data_dir.display());

    if !data_dir.exists() {
        fs::create_dir_all(data_dir).map_err(|err| anyhow!("Failed to create data dir {err:?}"))?;
    }
    let local_manga_path =
        resolve_path_option(cli.local_manga_path.as_ref(), data_dir, "local-manga");
    let local_manga_dir = PathBuf::from(&local_manga_path);
    if !local_manga_dir.exists()
        && let Err(err) = fs::create_dir_all(&local_manga_dir)
    {
        warn!(
            "Failed to create local manga dir {}: {err}",
            local_manga_dir.display()
        );
    }
    match run_local_manga_archive_shim(&local_manga_dir) {
        Ok(summary) => {
            if summary.scanned_archives > 0 {
                info!(
                    "📚 Local manga archive shim scanned {} archive(s), planned {} archive(s) across {} folder(s), copied {} archive(s), refreshed {} folder(s), skipped {} archive(s).",
                    summary.scanned_archives,
                    summary.planned_archives,
                    summary.planned_folders,
                    summary.copied_archives,
                    summary.refreshed_folders,
                    summary.skipped_archives
                );
            }
        }
        Err(err) => warn!("Local manga archive shim failed: {err}"),
    }

    let local_anime_dir = data_dir.join("local-anime");
    if !local_anime_dir.exists()
        && let Err(err) = fs::create_dir_all(&local_anime_dir)
    {
        warn!(
            "Failed to create local anime dir {}: {err}",
            local_anime_dir.display()
        );
    }
    let local_novel_dir = data_dir.join("local-novel");
    if !local_novel_dir.exists()
        && let Err(err) = fs::create_dir_all(&local_novel_dir)
    {
        warn!(
            "Failed to create local novel dir {}: {err}",
            local_novel_dir.display()
        );
    }
    let bin_dir = data_dir.join("bin");
    if !bin_dir.exists() {
        fs::create_dir_all(&bin_dir).map_err(|err| anyhow!("Failed to create bin dir {err:?}"))?;
    }

    migrate_suwayomi_extensions(data_dir);
    migrate_suwayomi_database(data_dir);
    migrate_suwayomi_settings(data_dir);

    info!("📦 Extracting assets...");
    let jar_name = "Suwayomi-Server.jar";
    let _ = extract_file(&bin_dir, jar_name, JAR_BYTES)
        .map_err(|err| anyhow!("Failed to extract {jar_name} {err:?}"))?;
    let jar_rel_path = PathBuf::from("bin").join(jar_name);

    #[cfg(feature = "embed-jre")]
    {
        let natives_dir = data_dir.join("natives");
        if !natives_dir.exists() {
            info!("📦 Extracting Native Libraries (JogAmp)...");
            fs::create_dir_all(&natives_dir)
                .map_err(|e| anyhow!("Failed to create natives dir: {e}"))?;

            extract_zip(NATIVES_BYTES, &natives_dir)
                .map_err(|e| anyhow!("Failed to extract natives: {e}"))?;
        }
    }

    info!("🔍 Resolving Java...");
    let java_exec =
        resolve_java(data_dir).map_err(|err| anyhow!("Failed to resolve java install {err:?}"))?;
    let java_home = java_exec
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(data_dir);

    info!("☕ Spawning Suwayomi...");
    let manatan_db_path = resolve_path_option(cli.db_path.as_ref(), data_dir, "manatan.sqlite");
    let manatan_migrate_path = cli
        .migrate_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let runtime_only = cli.runtime_only;
    if runtime_only {
        info!("Suwayomi runtime-only mode enabled");
    }

    let suwayomi_pid_path = data_dir.join("suwayomi.pid");
    cleanup_orphan_suwayomi(&suwayomi_pid_path);
    ensure_suwayomi_port_available(SUWAYOMI_HOST, SUWAYOMI_PORT)?;

    let mut suwayomi_proc = Command::new(&java_exec)
        .current_dir(data_dir)
        .env("JAVA_HOME", java_home)
        .env(
            "SUWAYOMI_RUNTIME_ONLY",
            if runtime_only { "true" } else { "false" },
        )
        .arg("-Dsuwayomi.tachidesk.config.server.initialOpenInBrowserEnabled=false")
        .arg("-Dsuwayomi.tachidesk.config.server.webUIEnabled=false")
        .arg("-Dsuwayomi.tachidesk.config.server.enableCookieApi=true")
        .arg(format!("-Dsuwayomi.runtimeOnly={runtime_only}"))
        .arg(format!(
            "-Dsuwayomi.tachidesk.config.server.rootDir={}",
            data_dir.display()
        ))
        .arg(format!(
            "-Dsuwayomi.tachidesk.config.server.ip={SUWAYOMI_HOST}"
        ))
        .arg(format!(
            "-Dsuwayomi.tachidesk.config.server.port={SUWAYOMI_PORT}"
        ))
        .arg(format!(
            "-Dsuwayomi.tachidesk.config.server.localAnimeSourcePath={}",
            local_anime_dir.display()
        ))
        .arg("-XX:+ExitOnOutOfMemoryError")
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg("--add-opens=java.desktop/sun.awt=ALL-UNNAMED")
        .arg("--add-opens=java.desktop/javax.swing=ALL-UNNAMED")
        .arg("-jar")
        .arg(&jar_rel_path)
        .kill_on_drop(true)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| anyhow!("Failed to launch suwayomi {err:?}"))?;

    if let Some(pid) = suwayomi_proc.id() {
        if let Err(err) = fs::write(&suwayomi_pid_path, pid.to_string()) {
            warn!(
                "Failed to write Suwayomi pid file {}: {err}",
                suwayomi_pid_path.display()
            );
        }
    } else {
        warn!(
            "Suwayomi PID unavailable; skipping pid file at {}",
            suwayomi_pid_path.display()
        );
    }

    let manatan_runtime_url = if runtime_only {
        if let Some(value) = cli.java_url.as_deref()
            && value != SUWAYOMI_HTTP_BASE_URL
        {
            warn!(
                "Ignoring MANATAN_JAVA_URL={} while runtime-only is enabled; using {}",
                value, SUWAYOMI_HTTP_BASE_URL
            );
        }
        SUWAYOMI_HTTP_BASE_URL.to_string()
    } else {
        cli.java_url
            .clone()
            .unwrap_or_else(|| SUWAYOMI_HTTP_BASE_URL.to_string())
    };
    let tracker_remote_search = cli.tracker_remote_search;
    let tracker_search_ttl_seconds = cli.tracker_search_ttl_seconds;
    let downloads_path = resolve_path_option(cli.downloads_path.as_ref(), data_dir, "downloads");
    let downloaded_manga_fallback_state = DownloadedMangaFallbackState {
        db_path: PathBuf::from(manatan_db_path.clone()),
        downloads_path: PathBuf::from(downloads_path.clone()),
    };
    let aidoku_index_url = cli.aidoku_index_url.clone().unwrap_or_default();
    let aidoku_enabled = cli.aidoku_enabled;
    let aidoku_cache_path = resolve_path_option(cli.aidoku_cache_path.as_ref(), data_dir, "aidoku");
    let local_anime_path =
        resolve_path_option(cli.local_anime_path.as_ref(), data_dir, "local-anime");
    let local_novel_path_str =
        resolve_path_option(cli.local_novel_path.as_ref(), data_dir, "local-novel");
    let manatan_config = ManatanServerConfig {
        host: host.to_string(),
        port,
        java_runtime_url: manatan_runtime_url.clone(),
        webview_enabled: true,
        aidoku_index_url,
        aidoku_enabled,
        aidoku_cache_path,
        db_path: manatan_db_path,
        migrate_path: manatan_migrate_path,
        tracker_remote_search,
        tracker_search_ttl_seconds,
        downloads_path,
        local_manga_path,
        local_anime_path,
    };
    let manatan_state = build_state(manatan_config)
        .await
        .map_err(|err| anyhow!("Failed to init Manatan server: {err}"))?;
    ensure_runtime_bridge_available(&manatan_runtime_url)
        .await
        .map_err(|err| anyhow!("Failed runtime bridge preflight: {err}"))?;
    let manatan_router = build_router_without_cors(manatan_state)
        .layer(middleware::from_fn_with_state(
            downloaded_manga_fallback_state,
            serve_downloaded_manga_fallback,
        ))
        .layer(middleware::from_fn(serialize_manga_chapters_requests))
        .layer(middleware::from_fn(rewrite_manga_pages_response));

    info!("🌍 Starting Web Interface at http://{}:{}", host, port);

    let ocr_router = manatan_ocr_server::create_router(data_dir.clone());
    let yomitan_router = manatan_yomitan_server::create_router(data_dir.clone());
    let audio_router = manatan_audio_server::create_router(data_dir.clone());
    let sync_router = manatan_sync_server::create_router(data_dir.clone());
    let novel_router =
        manatan_novel_server::create_router(data_dir.clone(), PathBuf::from(local_novel_path_str));
    let system_router = Router::new().route("/version", any(current_version_handler));

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            AUTHORIZATION,
            CONTENT_TYPE,
            ACCEPT,
            ORIGIN,
            ACCESS_CONTROL_ALLOW_ORIGIN,
            ACCESS_CONTROL_ALLOW_HEADERS,
            ACCESS_CONTROL_REQUEST_METHOD,
        ])
        .allow_credentials(true);

    let app = Router::new()
        .nest("/api/ocr", ocr_router)
        .nest("/api/audio", audio_router)
        .nest("/api/sync", sync_router)
        .nest("/api/novel", novel_router)
        .nest("/api/system", system_router)
        .nest("/api/yomitan", yomitan_router)
        .merge(manatan_router)
        .fallback(serve_react_app)
        .layer(cors);

    let listener_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&listener_addr)
        .await
        .map_err(|err| anyhow!("Failed to create main server socket: {err:?}"))?;

    let server_future = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = shutdown_signal.recv().await;
        info!("🛑 Shutdown signal received.");
    });

    info!("✅ Unified Server Running.");

    tokio::select! {
        _ = suwayomi_proc.wait() => { error!("❌ Suwayomi exited unexpectedly"); }
        _ = server_future => { info!("✅ Web server shutdown complete."); }
    }

    info!("🛑 terminating child processes...");

    if let Err(err) = suwayomi_proc.kill().await {
        error!("Error killing Suwayomi: {err}");
    }
    let _ = suwayomi_proc.wait().await;
    let _ = fs::remove_file(&suwayomi_pid_path);
    info!("   Suwayomi terminated.");

    Ok(())
}

async fn serve_react_app(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    if !path.is_empty()
        && let Some(content) = FrontendAssets::get(path)
    {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            [
                (axum::http::header::CONTENT_TYPE, mime.as_ref()),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            content.data,
        )
            .into_response();
    }

    if let Some(index) = FrontendAssets::get("index.html")
        && let Ok(html_string) = std::str::from_utf8(index.data.as_ref())
    {
        let fixed_html = html_string.replace("<head>", "<head><base href=\"/\" />");

        return (
            [
                (axum::http::header::CONTENT_TYPE, "text/html"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            fixed_html,
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "404 - Index.html missing").into_response()
}

async fn rewrite_manga_pages_response(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if !is_manga_pages_path(&path) {
        return next.run(req).await;
    }

    let response = next.run(req).await;

    if !response.status().is_success() || !is_json_response(response.headers()) {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let body_bytes = match to_bytes(body, MAX_PAGES_RESPONSE_REWRITE_BYTES).await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!("failed to buffer manga pages response for URL rewrite: {err}");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::empty())
                .unwrap_or_else(|_| Response::new(Body::empty()));
        }
    };

    let Some(rewritten_body) = rewrite_manga_pages_body(&body_bytes) else {
        return Response::from_parts(parts, Body::from(body_bytes));
    };

    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::TRANSFER_ENCODING);
    parts.headers.remove(header::CONTENT_ENCODING);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    Response::from_parts(parts, Body::from(rewritten_body))
}

async fn serialize_manga_chapters_requests(req: Request, next: Next) -> Response {
    let Some(manga_id) = manga_chapters_path_manga_id(req.uri().path()) else {
        return next.run(req).await;
    };

    let lock = {
        let locks = MANGA_CHAPTER_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = locks.lock().expect("manga chapter lock registry poisoned");
        locks
            .entry(manga_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    };

    let _guard = lock.lock().await;
    next.run(req).await
}

async fn serve_downloaded_manga_fallback(
    State(state): State<DownloadedMangaFallbackState>,
    req: Request,
    next: Next,
) -> Response {
    let endpoint = parse_manga_page_endpoint(req.uri().path());
    if let Some(endpoint) = endpoint {
        match tokio::task::spawn_blocking(move || downloaded_manga_response(&state, endpoint)).await
        {
            Ok(Some(downloaded_response)) => return downloaded_response,
            Ok(None) => {}
            Err(err) => {
                warn!("downloaded manga response task failed: {err}");
            }
        }
    }

    let response = next.run(req).await;
    if matches!(endpoint, Some(MangaPageEndpoint::Page { .. })) {
        return correct_manga_page_image_content_type(response).await;
    }

    response
}

fn downloaded_manga_response(
    state: &DownloadedMangaFallbackState,
    endpoint: MangaPageEndpoint,
) -> Option<Response> {
    let location = find_downloaded_chapter_location(state, endpoint)?;
    let manifest = read_download_manifest(&location.chapter_dir)?;

    if manifest.pages.is_empty() {
        return None;
    }

    match endpoint {
        MangaPageEndpoint::Pages {
            manga_id,
            chapter_index,
        } => downloaded_pages_response(
            manga_id,
            chapter_index,
            location.page_count.max(manifest.pages.len()),
            &location.cache_key,
        ),
        MangaPageEndpoint::Page {
            page_index,
            manga_id: _,
            chapter_index: _,
        } => downloaded_page_image_response(&location.chapter_dir, &manifest, page_index),
    }
}

fn downloaded_pages_response(
    manga_id: i64,
    chapter_index: i64,
    page_count: usize,
    cache_key: &str,
) -> Option<Response> {
    let pages = (0..page_count)
        .map(|page_index| {
            cache_busted_manga_page_url(&format!(
                "/api/v1/manga/{manga_id}/chapter/{chapter_index}/page/{page_index}?manatan_downloaded={cache_key}"
            ))
        })
        .collect::<Vec<_>>();
    let body = serde_json::to_vec(&serde_json::json!({ "pages": pages })).ok()?;

    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-manatan-downloaded-fallback", "1")
            .body(Body::from(body))
            .unwrap_or_else(|_| Response::new(Body::empty())),
    )
}

fn downloaded_page_image_response(
    chapter_dir: &Path,
    manifest: &DownloadManifest,
    page_index: usize,
) -> Option<Response> {
    let page_name = manifest.pages.get(page_index)?;
    let bytes = read_downloaded_page(chapter_dir, manifest, page_name)?;
    let content_type = content_type_for_downloaded_page(page_name, &bytes);

    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-manatan-downloaded-fallback", "1")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| Response::new(Body::empty())),
    )
}

fn find_downloaded_chapter_location(
    state: &DownloadedMangaFallbackState,
    endpoint: MangaPageEndpoint,
) -> Option<DownloadedChapterLocation> {
    let (manga_id, chapter_index) = match endpoint {
        MangaPageEndpoint::Pages {
            manga_id,
            chapter_index,
        }
        | MangaPageEndpoint::Page {
            manga_id,
            chapter_index,
            page_index: _,
        } => (manga_id, chapter_index),
    };

    let conn = rusqlite::Connection::open(&state.db_path).ok()?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT ch.id,
                   COALESCE(ch.page_count, 0),
                   m.source_id,
                   COALESCE(group_concat(DISTINCT c.name), '')
            FROM chapters ch
            JOIN manga m ON m.id = ch.manga_id
            LEFT JOIN manga_categories mc ON mc.manga_id = m.id
            LEFT JOIN categories c ON c.id = mc.category_id
            WHERE m.id = ?1 AND ch.source_order = ?2 AND ch.is_downloaded = 1
            GROUP BY ch.id, ch.page_count, m.source_id
            "#,
        )
        .ok()?;
    let row = stmt
        .query_row(rusqlite::params![manga_id, chapter_index], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .ok()?;

    let chapter_id = row.0;
    let db_page_count = usize::try_from(row.1).unwrap_or(0);
    let source_id = row.2;
    let categories = row
        .3
        .split(',')
        .map(str::trim)
        .filter(|category| !category.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let chapter_dir = find_downloaded_chapter_dir(
        &state.downloads_path,
        &categories,
        &source_id,
        manga_id,
        chapter_id,
    )?;
    let manifest = read_download_manifest(&chapter_dir)?;
    let page_count = if manifest.pages.is_empty() {
        db_page_count
    } else {
        manifest.pages.len()
    };

    Some(DownloadedChapterLocation {
        chapter_dir,
        page_count,
        cache_key: format!("c{chapter_id}-p{page_count}"),
    })
}

fn find_downloaded_chapter_dir(
    downloads_path: &Path,
    categories: &[String],
    source_id: &str,
    manga_id: i64,
    chapter_id: i64,
) -> Option<PathBuf> {
    let root = downloads_path.join("mangas");
    let source_suffix = format!("--s{source_id}");
    let manga_suffix = format!("--m{manga_id}");
    let chapter_suffix = format!("--c{chapter_id}");

    for category in categories {
        let category_dir = root.join(category);
        if let Some(chapter_dir) = find_chapter_dir_under_category(
            &category_dir,
            &source_suffix,
            &manga_suffix,
            &chapter_suffix,
        ) {
            return Some(chapter_dir);
        }
    }

    for category_dir in fs::read_dir(root).ok()?.filter_map(Result::ok) {
        let Ok(file_type) = category_dir.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if let Some(chapter_dir) = find_chapter_dir_under_category(
            &category_dir.path(),
            &source_suffix,
            &manga_suffix,
            &chapter_suffix,
        ) {
            return Some(chapter_dir);
        }
    }

    None
}

fn find_chapter_dir_under_category(
    category_dir: &Path,
    source_suffix: &str,
    manga_suffix: &str,
    chapter_suffix: &str,
) -> Option<PathBuf> {
    let source_dir = find_child_dir_by_suffix(category_dir, source_suffix)?;
    let manga_dir = find_child_dir_by_suffix(&source_dir, manga_suffix)?;
    find_child_dir_by_suffix(&manga_dir, chapter_suffix)
}

fn find_child_dir_by_suffix(parent: &Path, suffix: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(parent).ok()?.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() && entry.file_name().to_string_lossy().ends_with(suffix) {
            return Some(entry.path());
        }
    }

    None
}

fn read_download_manifest(chapter_dir: &Path) -> Option<DownloadManifest> {
    let manifest_path = chapter_dir.join("manifest.json");
    let bytes = fs::read(manifest_path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn read_downloaded_page(
    chapter_dir: &Path,
    manifest: &DownloadManifest,
    page_name: &str,
) -> Option<Vec<u8>> {
    if let Some(relative_page_path) = safe_relative_manifest_path(page_name) {
        let page_path = chapter_dir.join(relative_page_path);
        if page_path.is_file()
            && let Ok(bytes) = fs::read(page_path)
        {
            return Some(bytes);
        }
    }

    let archive_name = manifest.archive_file.as_deref().unwrap_or("chapter.cbz");
    let archive_relative_path = safe_relative_manifest_path(archive_name)?;
    let archive_file = fs::File::open(chapter_dir.join(archive_relative_path)).ok()?;
    let mut archive = zip::ZipArchive::new(archive_file).ok()?;
    let mut page = archive.by_name(page_name).ok()?;
    let mut bytes = Vec::new();
    page.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn safe_relative_manifest_path(path: &str) -> Option<&Path> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }

    Some(path)
}

fn content_type_for_downloaded_page(page_name: &str, bytes: &[u8]) -> &'static str {
    if let Some(content_type) = sniff_image_content_type(bytes) {
        return content_type;
    }
    if page_name.to_ascii_lowercase().ends_with(".avif") {
        return "image/avif";
    }

    "application/octet-stream"
}

async fn correct_manga_page_image_content_type(response: Response) -> Response {
    if !response.status().is_success() || is_image_response(response.headers()) {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let body_bytes = match to_bytes(body, MAX_PAGE_IMAGE_CONTENT_TYPE_REWRITE_BYTES).await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!("failed to buffer manga page image response for content-type correction: {err}");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::empty())
                .unwrap_or_else(|_| Response::new(Body::empty()));
        }
    };

    let Some(content_type) = sniff_image_content_type(&body_bytes) else {
        return Response::from_parts(parts, Body::from(body_bytes));
    };

    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::TRANSFER_ENCODING);
    parts.headers.remove(header::CONTENT_ENCODING);
    parts
        .headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    parts
        .headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    parts.headers.insert(
        "x-manatan-image-content-type-fix",
        HeaderValue::from_static("1"),
    );

    Response::from_parts(parts, Body::from(body_bytes))
}

fn is_image_response(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().starts_with("image/"))
        .unwrap_or(false)
}

fn sniff_image_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }

    None
}

fn is_json_response(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("application/json"))
        .unwrap_or(false)
}

fn rewrite_manga_pages_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut value = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let pages = value.get_mut("pages")?.as_array_mut()?;
    let mut changed = false;

    for page in pages {
        let Some(raw_url) = page.as_str() else {
            continue;
        };
        let Some(rewritten_url) = rewrite_manga_page_url(raw_url) else {
            continue;
        };
        if rewritten_url != raw_url {
            *page = serde_json::Value::String(rewritten_url);
            changed = true;
        }
    }

    changed.then(|| serde_json::to_vec(&value).ok()).flatten()
}

fn rewrite_manga_page_url(raw_url: &str) -> Option<String> {
    if raw_url.starts_with('/') {
        return is_manga_page_path(raw_url).then(|| cache_busted_manga_page_url(raw_url));
    }

    let parsed = Url::parse(raw_url).ok()?;
    let host = parsed.host_str()?;
    if !is_loopback_or_unspecified_host(host) || !is_manga_page_path(parsed.path()) {
        return None;
    }

    let mut path_and_query = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    Some(cache_busted_manga_page_url(&path_and_query))
}

fn cache_busted_manga_page_url(path_and_query: &str) -> String {
    if path_and_query.contains("manatan_page_cache=") {
        return path_and_query.to_string();
    }

    let separator = if path_and_query.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{path_and_query}{separator}manatan_page_cache={MANGA_PAGE_CACHE_BUSTER}")
}

fn is_manga_pages_path(path: &str) -> bool {
    let mut segments = path.trim_start_matches('/').split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ),
        (
            Some("api"),
            Some("v1"),
            Some("manga"),
            Some(_),
            Some("chapter"),
            Some(_),
            Some("pages"),
            None,
        )
    )
}

fn parse_manga_page_endpoint(path: &str) -> Option<MangaPageEndpoint> {
    let mut segments = path.trim_start_matches('/').split('/');
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (
            Some("api"),
            Some("v1"),
            Some("manga"),
            Some(manga_id),
            Some("chapter"),
            Some(chapter_index),
            Some("pages"),
            None,
            None,
        ) => Some(MangaPageEndpoint::Pages {
            manga_id: manga_id.parse().ok()?,
            chapter_index: chapter_index.parse().ok()?,
        }),
        (
            Some("api"),
            Some("v1"),
            Some("manga"),
            Some(manga_id),
            Some("chapter"),
            Some(chapter_index),
            Some("page"),
            Some(page_index),
            None,
        ) => Some(MangaPageEndpoint::Page {
            manga_id: manga_id.parse().ok()?,
            chapter_index: chapter_index.parse().ok()?,
            page_index: page_index.parse().ok()?,
        }),
        _ => None,
    }
}

fn manga_chapters_path_manga_id(path: &str) -> Option<&str> {
    let mut segments = path.trim_start_matches('/').split('/');
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (Some("api"), Some("v1"), Some("manga"), Some(manga_id), Some("chapters"), None) => {
            Some(manga_id)
        }
        _ => None,
    }
}

fn is_manga_page_path(path: &str) -> bool {
    let mut segments = path.trim_start_matches('/').split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ),
        (
            Some("api"),
            Some("v1"),
            Some("manga"),
            Some(_),
            Some("chapter"),
            Some(_),
            Some("page"),
            Some(_),
            None,
        )
    )
}

fn is_loopback_or_unspecified_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "0.0.0.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_loopback_manga_page_urls_to_relative_paths() {
        let body = br#"{"pages":["http://127.0.0.1:4568/api/v1/manga/1/chapter/2/page/0","http://localhost:4568/api/v1/manga/1/chapter/2/page/1?cache=true"]}"#;

        let rewritten = rewrite_manga_pages_body(body).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();

        assert_eq!(
            value["pages"][0],
            "/api/v1/manga/1/chapter/2/page/0?manatan_page_cache=downloadfix2"
        );
        assert_eq!(
            value["pages"][1],
            "/api/v1/manga/1/chapter/2/page/1?cache=true&manatan_page_cache=downloadfix2"
        );
    }

    #[test]
    fn adds_cache_buster_to_relative_manga_page_urls() {
        let body = br#"{"pages":["/api/v1/manga/1/chapter/2/page/0"]}"#;

        let rewritten = rewrite_manga_pages_body(body).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();

        assert_eq!(
            value["pages"][0],
            "/api/v1/manga/1/chapter/2/page/0?manatan_page_cache=downloadfix2"
        );
    }

    #[test]
    fn leaves_non_loopback_and_non_page_urls_unchanged() {
        let body = br#"{"pages":["https://cdn.example.test/api/v1/manga/1/chapter/2/page/0","http://127.0.0.1:4568/api/v1/manga/1/chapter/2/thumbnail"]}"#;

        assert!(rewrite_manga_pages_body(body).is_none());
    }

    #[test]
    fn does_not_duplicate_manga_page_cache_buster() {
        assert_eq!(
            cache_busted_manga_page_url(
                "/api/v1/manga/1/chapter/2/page/0?manatan_page_cache=downloadfix2"
            ),
            "/api/v1/manga/1/chapter/2/page/0?manatan_page_cache=downloadfix2"
        );
    }

    #[test]
    fn sniffs_image_content_types_from_bytes() {
        assert_eq!(
            sniff_image_content_type(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(
            sniff_image_content_type(b"RIFFxxxxWEBP"),
            Some("image/webp")
        );
        assert_eq!(sniff_image_content_type(b"PK\x03\x04"), None);
    }

    #[test]
    fn sorts_local_manga_archive_entries_naturally() {
        let mut entries = vec![
            "page10.webp".to_string(),
            "page2.webp".to_string(),
            "page001.webp".to_string(),
        ];

        entries.sort_by(|left, right| natural_cmp(left, right));

        assert_eq!(entries, vec!["page001.webp", "page2.webp", "page10.webp"]);
    }

    #[test]
    fn derives_local_manga_title_from_chapter_archive_stem() {
        assert_eq!(
            title_from_archive_stem("Chapter 001 - Example Manga 001"),
            "Example Manga"
        );
        assert_eq!(
            title_from_archive_stem("Chapter 10 - Example Manga 11"),
            "Example Manga 11"
        );
    }

    #[test]
    fn prefers_local_manga_meta_json_title() {
        let bytes = br#"{"title":{"japanese":"Japanese Name","english":"English Name"}}"#;

        assert_eq!(
            local_manga_title_from_meta_json(bytes),
            Some("Japanese Name".to_string())
        );
    }

    #[test]
    fn matches_only_page_list_endpoint_for_response_rewrite() {
        assert!(is_manga_pages_path("/api/v1/manga/1/chapter/2/pages"));
        assert!(!is_manga_pages_path("/api/v1/manga/1/chapter/2/pages/0"));
        assert!(!is_manga_pages_path("/api/v1/manga/1/chapter/2/page/0"));
    }

    #[test]
    fn parses_downloaded_manga_fallback_endpoints() {
        assert_eq!(
            parse_manga_page_endpoint("/api/v1/manga/708094028058387/chapter/1/pages"),
            Some(MangaPageEndpoint::Pages {
                manga_id: 708094028058387,
                chapter_index: 1
            })
        );
        assert_eq!(
            parse_manga_page_endpoint("/api/v1/manga/708094028058387/chapter/1/page/0"),
            Some(MangaPageEndpoint::Page {
                manga_id: 708094028058387,
                chapter_index: 1,
                page_index: 0
            })
        );
        assert_eq!(
            parse_manga_page_endpoint("/api/v1/manga/not-a-number/chapter/1/page/0"),
            None
        );
    }

    #[test]
    fn extracts_manga_id_from_chapters_endpoint() {
        assert_eq!(
            manga_chapters_path_manga_id("/api/v1/manga/2779780616136912/chapters"),
            Some("2779780616136912")
        );
        assert_eq!(
            manga_chapters_path_manga_id("/api/v1/manga/2779780616136912/chapter/4/pages"),
            None
        );
        assert_eq!(
            manga_chapters_path_manga_id("/api/v1/manga/2779780616136912/chapters/extra"),
            None
        );
    }

    #[test]
    fn matches_page_image_endpoint_for_url_rewrite() {
        assert!(is_manga_page_path("/api/v1/manga/1/chapter/2/page/0"));
        assert!(!is_manga_page_path("/api/v1/manga/1/chapter/2/page"));
        assert!(!is_manga_page_path(
            "/api/v1/manga/1/chapter/2/page/0/extra"
        ));
        assert!(!is_manga_page_path("/api/v1/manga/1/chapter/2/pages"));
    }
}

fn ensure_suwayomi_port_available(host: &str, port: u16) -> anyhow::Result<()> {
    match TcpListener::bind((host, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(err) => Err(anyhow!(
            "{host}:{port} is already in use ({err}). Stop any existing Suwayomi/Manatan process and try again."
        )),
    }
}

async fn ensure_runtime_bridge_available(base_url: &str) -> anyhow::Result<()> {
    let client = Client::new();
    let health_url = format!("{base_url}/runtime/v1/health");
    let bridge_url = format!("{base_url}/runtime/v1/bridge/manga/pages");

    for _ in 0..60 {
        if let Ok(resp) = client.get(&health_url).send().await
            && resp.status().is_success()
        {
            let bridge_resp = client
                .post(&bridge_url)
                .header("content-type", "application/json")
                .body("{}")
                .send()
                .await;

            return match bridge_resp {
                Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "[failed to read body]".to_string());
                    Err(anyhow!(
                        "runtime bridge endpoint missing at {bridge_url} (status 404, body={body}). This usually means an outdated or wrong Suwayomi runtime is running."
                    ))
                }
                Ok(_) => Ok(()),
                Err(err) => Err(anyhow!("failed calling runtime bridge endpoint: {err}")),
            };
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(anyhow!(
        "timed out waiting for runtime health endpoint {health_url}"
    ))
}

fn get_asset_target_string() -> &'static str {
    #[cfg(target_os = "windows")]
    return "Windows-x64";

    #[cfg(target_os = "macos")]
    {
        #[cfg(target_arch = "aarch64")]
        return "macOS-Silicon";
        #[cfg(target_arch = "x86_64")]
        return "macOS-Intel";
    }

    #[cfg(target_os = "linux")]
    {
        #[cfg(target_arch = "aarch64")]
        return "Linux-arm64.tar";

        #[cfg(target_arch = "x86_64")]
        return "Linux-amd64.tar";
    }
}

fn cleanup_orphan_suwayomi(pid_path: &Path) {
    let Some(pid) = read_pid_file(pid_path) else {
        return;
    };

    #[cfg(unix)]
    {
        if !is_suwayomi_process(pid) {
            warn!(
                "Stale pid file {} does not match Suwayomi process; removing.",
                pid_path.display()
            );
            let _ = fs::remove_file(pid_path);
            return;
        }

        if !is_process_alive(pid) {
            let _ = fs::remove_file(pid_path);
            return;
        }

        info!("Found leftover Suwayomi process (pid {pid}). Shutting it down...");
        terminate_process(pid, Duration::from_secs(5));
        let _ = fs::remove_file(pid_path);
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        let _ = fs::remove_file(pid_path);
    }
}

fn read_pid_file(pid_path: &Path) -> Option<i32> {
    let contents = fs::read_to_string(pid_path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        let _ = fs::remove_file(pid_path);
        return None;
    }
    match trimmed.parse::<i32>() {
        Ok(pid) => Some(pid),
        Err(_) => {
            let _ = fs::remove_file(pid_path);
            None
        }
    }
}

#[cfg(unix)]
fn is_process_alive(pid: i32) -> bool {
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    err.raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn is_suwayomi_process(pid: i32) -> bool {
    let cmdline_path = format!("/proc/{pid}/cmdline");
    let Ok(bytes) = fs::read(cmdline_path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes).replace('\0', " ");
    text.contains("Suwayomi-Server.jar")
}

#[cfg(unix)]
fn terminate_process(pid: i32, timeout: Duration) {
    let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if !is_process_alive(pid) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
}

fn check_for_updates(status: Arc<Mutex<UpdateStatus>>) {
    *status.lock().expect("lock shouldn't panic") = UpdateStatus::Checking;

    // We use the same configuration for checking as we do for updating
    // This ensures we only "find" releases that actually match our custom asset naming
    let target_str = get_asset_target_string();

    let updater_result = build_updater(REPO_NAME, target_str);

    match updater_result {
        Ok(updater) => {
            match updater.get_latest_release() {
                Ok(release) => {
                    // Check if remote version > local version
                    let is_newer =
                        self_update::version::bump_is_greater(APP_VERSION, &release.version)
                            .unwrap_or(false);

                    if is_newer {
                        *status.lock().expect("lock shouldn't panic") =
                            UpdateStatus::UpdateAvailable(release.version);
                    } else {
                        *status.lock().expect("lock shouldn't panic") = UpdateStatus::UpToDate;
                    }
                }
                Err(e) => {
                    if let Ok(legacy_updater) = build_updater(LEGACY_REPO_NAME, target_str) {
                        match legacy_updater.get_latest_release() {
                            Ok(release) => {
                                let is_newer = self_update::version::bump_is_greater(
                                    APP_VERSION,
                                    &release.version,
                                )
                                .unwrap_or(false);

                                if is_newer {
                                    *status.lock().expect("lock shouldn't panic") =
                                        UpdateStatus::UpdateAvailable(release.version);
                                } else {
                                    *status.lock().expect("lock shouldn't panic") =
                                        UpdateStatus::UpToDate;
                                }
                            }
                            Err(err) => {
                                *status.lock().expect("lock shouldn't panic") =
                                    UpdateStatus::Error(err.to_string())
                            }
                        }
                    } else {
                        *status.lock().expect("lock shouldn't panic") =
                            UpdateStatus::Error(e.to_string())
                    }
                }
            }
        }
        Err(e) => {
            if let Ok(legacy_updater) = build_updater(LEGACY_REPO_NAME, target_str) {
                match legacy_updater.get_latest_release() {
                    Ok(release) => {
                        let is_newer =
                            self_update::version::bump_is_greater(APP_VERSION, &release.version)
                                .unwrap_or(false);

                        if is_newer {
                            *status.lock().expect("lock shouldn't panic") =
                                UpdateStatus::UpdateAvailable(release.version);
                        } else {
                            *status.lock().expect("lock shouldn't panic") = UpdateStatus::UpToDate;
                        }
                    }
                    Err(err) => {
                        *status.lock().expect("lock shouldn't panic") =
                            UpdateStatus::Error(err.to_string())
                    }
                }
            } else {
                *status.lock().expect("lock shouldn't panic") = UpdateStatus::Error(e.to_string())
            }
        }
    }
}

fn perform_update() -> Result<(), Box<dyn std::error::Error>> {
    let target_str = get_asset_target_string();

    if let Ok(updater) = build_updater_with_download(REPO_NAME, target_str)
        && updater.update().is_ok()
    {
        return Ok(());
    }

    build_updater_with_download(LEGACY_REPO_NAME, target_str)?.update()?;

    Ok(())
}

fn build_updater(
    repo_name: &str,
    target_str: &str,
) -> Result<Box<dyn ReleaseUpdate>, self_update::errors::Error> {
    self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(repo_name)
        .bin_name(BIN_NAME)
        .target(target_str)
        .current_version(APP_VERSION)
        .build()
}

fn build_updater_with_download(
    repo_name: &str,
    target_str: &str,
) -> Result<Box<dyn ReleaseUpdate>, self_update::errors::Error> {
    self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(repo_name)
        .bin_name(BIN_NAME)
        .target(target_str)
        .show_download_progress(true)
        .current_version(APP_VERSION)
        .no_confirm(true)
        .build()
}

async fn open_webpage_when_ready(host: Ipv4Addr, port: u16) {
    let client = Client::new();

    let host_target = if host == Ipv4Addr::new(0, 0, 0, 0) {
        "localhost".to_string()
    } else {
        host.to_string()
    };
    let url = format!("http://{host_target}:{port}");
    let health_url = format!("http://{host_target}:{port}/health");

    info!("⏳ Polling health endpoint for readiness (timeout 10s)...");

    // Define the polling task
    let polling_task = async {
        loop {
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    info!("✅ Server is responsive! Opening browser...");
                    if let Err(e) = open::that(&url) {
                        error!("❌ Failed to open browser: {}", e);
                    }
                    return;
                }
                err => {
                    warn!("Failed to poll health to open webpage: {err:?}");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    };

    if tokio::time::timeout(Duration::from_secs(10), polling_task)
        .await
        .is_err()
    {
        error!("❌ Timed out waiting for server readiness (10s). Browser open cancelled.");
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm = signal(SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = async {
                if let Some(sigterm) = &mut sigterm {
                    sigterm.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn current_version_handler() -> impl IntoResponse {
    axum::Json(VersionResponse {
        version: APP_VERSION.to_string(),
        variant: "desktop".to_string(), // Frontend will see 'desktop' and HIDE the button
    })
}

fn is_flatpak() -> bool {
    std::env::var("FLATPAK_ID").is_ok()
}
