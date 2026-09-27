use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use tayoreader_core::fetch::Fetcher;
use tayoreader_core::model::{Entry, EntryFilter, Feed};
use tayoreader_core::service::{RefreshOutcome, Service};
use tayoreader_core::store::Store;

#[derive(Parser)]
#[command(name = "tayoreader", version, about = "TayoReader: RSS リーダー")]
struct Cli {
    /// DB ファイルのパス（省略時は OS のアプリデータディレクトリ）
    #[arg(long, global = true, value_name = "PATH")]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// フィードを購読する
    Add {
        url: String,
        /// 所属フォルダ（複数指定可）
        #[arg(long = "folder", value_name = "NAME")]
        folders: Vec<String>,
    },
    /// 購読を解除する（記事も削除する）
    Remove { feed_id: i64 },
    /// 購読一覧を表示する
    Feeds,
    /// OPML から購読を取り込む（記事の取得はしない）
    ImportOpml { file: PathBuf },
    /// フィードを取得する（省略時は全フィード）
    Refresh { feed_id: Option<i64> },
    /// 記事一覧を新しい順に表示する
    Entries {
        #[arg(long)]
        unread: bool,
        #[arg(long)]
        starred: bool,
        #[arg(long, value_name = "FEED_ID")]
        feed: Option<i64>,
        #[arg(long, value_name = "NAME")]
        folder: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// タイトル・本文を検索する（空白で区切った語をすべて含む記事）
        #[arg(long, value_name = "QUERY")]
        search: Option<String>,
    },
    /// 既読にする
    Read {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// 未読にする
    Unread {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// スターを付ける
    Star {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// スターを外す
    Unstar {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let db = match cli.db {
        Some(path) => path,
        None => tayoreader_core::default_db_path()
            .context("could not determine the application data directory")?,
    };
    let store =
        Store::open(&db).with_context(|| format!("failed to open database {}", db.display()))?;
    let service = Service::new(store, Fetcher::new()?);
    let store = service.store();

    match cli.command {
        Command::Add { url, folders } => {
            let added = service.add_feed(&url, &folders).await?;
            println!(
                "added feed {}: {} ({} entries)",
                added.feed_id,
                added.title.as_deref().unwrap_or(&url),
                added.new_entries
            );
        }
        Command::Remove { feed_id } => {
            store.remove_feed(feed_id)?;
            println!("removed feed {feed_id}");
        }
        Command::Feeds => {
            for feed in store.list_feeds()? {
                print_feed(&feed);
            }
        }
        Command::ImportOpml { file } => {
            let xml = std::fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let report = service.import_opml(&xml).await?;
            println!(
                "imported {} feeds ({} already subscribed)",
                report.added, report.merged
            );
        }
        Command::Refresh { feed_id } => {
            let report = service.refresh(feed_id).await?;
            for failure in report.failures() {
                if let RefreshOutcome::Failed(message) = &failure.outcome {
                    eprintln!("failed: [{}] {}: {message}", failure.feed_id, failure.url);
                }
            }
            let failed = report.failures().count();
            println!(
                "refreshed {} feeds: {} new entries, {failed} failed",
                report.feeds.len(),
                report.new_entries()
            );
            if failed > 0 {
                return Ok(ExitCode::FAILURE);
            }
        }
        Command::Entries {
            unread,
            starred,
            feed,
            folder,
            limit,
            search,
        } => {
            let filter = EntryFilter {
                unread_only: unread,
                starred_only: starred,
                feed_id: feed,
                folder,
                limit: Some(limit),
                offset: None,
                query: search,
            };
            for entry in store.list_entries(&filter)? {
                print_entry(&entry);
            }
        }
        Command::Read { ids } => print_updated(store.set_read(&ids, true)?, ids.len()),
        Command::Unread { ids } => print_updated(store.set_read(&ids, false)?, ids.len()),
        Command::Star { ids } => print_updated(store.set_starred(&ids, true)?, ids.len()),
        Command::Unstar { ids } => print_updated(store.set_starred(&ids, false)?, ids.len()),
    }
    Ok(ExitCode::SUCCESS)
}

fn print_feed(feed: &Feed) {
    let title = feed.title.as_deref().unwrap_or("(no title)");
    let folders = if feed.folders.is_empty() {
        String::new()
    } else {
        format!("  [{}]", feed.folders.join(", "))
    };
    let error = feed
        .last_error
        .as_deref()
        .map(|e| format!("  ! {e}"))
        .unwrap_or_default();
    println!("{:>5}  {title}  <{}>{folders}{error}", feed.id, feed.url);
}

fn print_entry(entry: &Entry) {
    let unread = if entry.is_read { ' ' } else { '●' };
    let star = if entry.is_starred { '★' } else { ' ' };
    let date = format_time(entry.published_at.unwrap_or(entry.fetched_at));
    let title = entry.title.as_deref().unwrap_or("(no title)");
    println!("{:>6} {unread}{star} {date}  {title}", entry.id);
}

fn print_updated(updated: usize, requested: usize) {
    println!("updated {updated} of {requested} entries");
}

fn format_time(unix: i64) -> String {
    DateTime::from_timestamp(unix, 0)
        .map(|d| d.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
