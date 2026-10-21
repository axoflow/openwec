use anyhow::{anyhow, bail, Context, Result};
use clap::ArgMatches;
use common::{bookmark::BookmarkData, database::Db, subscription::SubscriptionData};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, Read, Write},
};

use crate::utils;

pub async fn run(db: &Db, matches: &ArgMatches) -> Result<()> {
    match matches.subcommand() {
        Some(("show", matches)) => {
            show(db, matches).await?;
        }
        Some(("delete", matches)) => {
            delete(db, matches).await?;
        }
        Some(("copy", matches)) => {
            copy(db, matches).await?;
        }
        Some(("export", matches)) => {
            export(db, matches).await?;
        }
        Some(("import", matches)) => {
            import(db, matches).await?;
        }
        _ => {
            bail!("Invalid subcommand")
        }
    };
    Ok(())
}

async fn show(db: &Db, matches: &ArgMatches) -> Result<()> {
    let machine = matches.get_one::<String>("machine").cloned();
    let subscription_identifier = matches
        .get_one::<String>("subscription")
        .expect("Required by clap")
        .to_string();

    let subscription = utils::find_subscription(db, &subscription_identifier)
        .await?
        .ok_or_else(|| {
            anyhow!(
                "Could not find subscription with identifier {}",
                subscription_identifier
            )
        })?;

    if let Some(machine) = machine {
        let bookmark = db
            .get_bookmark(&machine, &subscription.uuid_string())
            .await?;
        match bookmark {
            Some(str) => {
                println!("{}", str)
            }
            None => bail!(
                "No bookmarks found for {} within subscription \"{}\"",
                machine,
                subscription.name()
            ),
        };
    } else {
        for data in db.get_bookmarks(&subscription.uuid_string()).await? {
            println!("{}:{}", data.machine, data.bookmark);
        }
    }

    Ok(())
}

async fn delete(db: &Db, matches: &ArgMatches) -> Result<()> {
    let machine = matches.get_one::<String>("machine").cloned();
    let subscription_identifier = matches.get_one::<String>("subscription").cloned();

    let subscription = match subscription_identifier {
        Some(identifier) => Some(
            utils::find_subscription(db, &identifier)
                .await?
                .ok_or_else(|| {
                    anyhow!("Could not find subscription with identifier {}", identifier)
                })?,
        ),
        None => None,
    };

    let message = match (&machine, &subscription) {
        (Some(machine), Some(subscription)) => format!("You are about to delete the bookmark of {} within subscription \"{}\".\nWARNING: You may lose logs!\nAre you sure?", machine, subscription.name()),
        (Some(machine), None) => format!("You are about to delete all bookmarks stored for {}.\nWARNING: You may lose logs!\nAre you sure?", machine),
        (None, Some(subscription)) => format!("You are about to delete all bookmarks stored for subscription \"{}\".\nWARNING: You may lose logs!\nAre you sure?", subscription.name()),
        (None, None) => "You are about to delete all stored bookmarks.\nWARNING: You may lose logs!\nAre you sure?".to_string(),
    };

    if utils::confirm(&message) {
        let uuid_opt = subscription.map(|x| x.uuid_string());
        db.delete_bookmarks(machine.as_deref(), uuid_opt.as_deref())
            .await?;
        println!("Done");
    } else {
        println!("Aborted");
    }

    Ok(())
}

async fn copy(db: &Db, matches: &ArgMatches) -> Result<()> {
    let machine = matches.get_one::<String>("machine").cloned();
    let source_id = matches
        .get_one::<String>("source")
        .expect("Required by clap")
        .to_string();
    let dest_id = matches
        .get_one::<String>("destination")
        .expect("Required by clap")
        .to_string();

    if source_id == dest_id {
        bail!("Source and destination are equal");
    }

    let source = utils::find_subscription(db, &source_id)
        .await?
        .ok_or_else(|| {
            anyhow!(
                "Could not find source subscription with identifier \"{}\"",
                source_id
            )
        })?;
    let destination = utils::find_subscription(db, &dest_id)
        .await?
        .ok_or_else(|| {
            anyhow!(
                "Could not find destination subscription with identifier \"{}\"",
                dest_id
            )
        })?;

    if let Some(machine) = machine {
        let existing_bookmark = db
            .get_bookmark(&machine, &destination.uuid_string())
            .await?;
        if existing_bookmark.is_some() {
            println!(
                "WARNING: A bookmark for {} already exists within subscription \"{}\"",
                machine,
                destination.name()
            )
        }

        let bookmark = db
            .get_bookmark(&machine, &source.uuid_string())
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "Could not find bookmark for {} in \"{}\"",
                    machine,
                    source.name()
                )
            })?;

        if !utils::confirm(format!("You are about to copy bookmark for {} in \"{}\" to \"{}\".\nWARNING: You may lose logs!\nAre you sure?", machine, source.name(), destination.name()).as_str()) {
            println!("Aborted");
            return Ok(());
        }

        db.store_bookmark(&machine, &destination.uuid_string(), &bookmark)
            .await?;
        println!("1 bookmark copied");
    } else {
        if !utils::confirm(format!("You are about to copy all bookmarks of subscription \"{}\" to \"{}\".\nWARNING: You may lose logs!\nAre you sure?", source.name(), destination.name()).as_str()) {
            println!("Aborted");
            return Ok(())
        };

        let mut counter: usize = 0;
        for data in db.get_bookmarks(&source.uuid_string()).await? {
            db.store_bookmark(&data.machine, &destination.uuid_string(), &data.bookmark)
                .await?;
            counter += 1;
        }
        println!("{} bookmarks copied", counter);
    };

    Ok(())
}

async fn export(db: &Db, matches: &ArgMatches) -> Result<()> {
    let path = matches
        .get_one::<String>("file")
        .expect("defaulted by clap");
    let subscriptions = utils::find_subscriptions(db, matches, "subscription").await?;
    let mut bookmarks = Vec::new();
    for subscription in subscriptions {
        bookmarks.extend(db.get_bookmarks(&subscription.uuid_string()).await?);
    }
    let count = bookmarks.len();
    write_bookmarks(path, &bookmarks)?;
    if path != "-" {
        println!("{} bookmarks exported to {}", count, path);
    }
    Ok(())
}

async fn import(db: &Db, matches: &ArgMatches) -> Result<()> {
    let path = matches.get_one::<String>("file").expect("Required by clap");
    let bookmarks = read_bookmarks(path)?;
    let imported = import_bookmarks(db, bookmarks).await?;
    println!("{} bookmarks imported", imported);
    Ok(())
}

/// Writes bookmarks as JSON to `path`, or to stdout if `path` is "-".
fn write_bookmarks(path: &str, bookmarks: &[BookmarkData]) -> Result<()> {
    let content = serde_json::to_string_pretty(bookmarks)?;
    if path == "-" {
        println!("{}", content);
    } else {
        let mut file = File::create(path).with_context(|| format!("Failed to create {}", path))?;
        writeln!(file, "{}", content).with_context(|| format!("Failed to write {}", path))?;
    }
    Ok(())
}

/// Reads bookmarks as JSON from `path`, or from stdin if `path` is "-".
fn read_bookmarks(path: &str) -> Result<Vec<BookmarkData>> {
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(File::open(path).with_context(|| format!("Failed to open {}", path))?)
    };
    serde_json::from_reader(BufReader::new(reader))
        .with_context(|| format!("Failed to parse {}", describe(path)))
}

fn describe(path: &str) -> &str {
    if path == "-" {
        "standard input"
    } else {
        path
    }
}

async fn import_bookmarks(db: &Db, bookmarks: Vec<BookmarkData>) -> Result<usize> {
    let mut subscriptions: HashMap<String, Option<SubscriptionData>> = HashMap::new();
    let mut imported = 0;
    for data in bookmarks {
        if !subscriptions.contains_key(&data.subscription) {
            let found = utils::find_subscription(db, &data.subscription).await?;
            subscriptions.insert(data.subscription.clone(), found);
        }
        let Some(subscription) = &subscriptions[&data.subscription] else {
            println!(
                "WARNING: Skipping {}: no subscription {}",
                data.machine, data.subscription
            );
            continue;
        };
        db.store_bookmark(&data.machine, &subscription.uuid_string(), &data.bookmark)
            .await?;
        imported += 1;
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::database::schema::{sqlite::register_migrations, Migrator};
    use common::database::sqlite::SQLiteDatabase;
    use std::sync::Arc;

    async fn migrated_db() -> Result<(Db, tempfile::TempPath)> {
        let path = tempfile::NamedTempFile::new()?.into_temp_path();
        let mut sqlite = SQLiteDatabase::new(path.to_str().unwrap()).await?;
        register_migrations(&mut sqlite);
        let db: Db = Arc::new(sqlite);
        db.setup_schema().await?;
        Migrator::new(db.clone()).up(None, false).await?;
        Ok((db, path))
    }

    fn data(machine: &str, subscription: &str, bookmark: &str) -> BookmarkData {
        BookmarkData {
            machine: machine.to_string(),
            subscription: subscription.to_string(),
            bookmark: bookmark.to_string(),
        }
    }

    #[tokio::test]
    async fn imports_by_subscription_name_or_uuid_and_exports_the_same() -> Result<()> {
        let (db, _path) = migrated_db().await?;
        let subscription = SubscriptionData::new("sub", "query");
        db.store_subscription(&subscription).await?;
        let uuid = subscription.uuid_string();

        let imported = import_bookmarks(
            &db,
            vec![
                data("DC01$@CORP.EXAMPLE.COM", "sub", "<BookmarkList/>"),
                data(
                    "dc02.corp.example.com",
                    &uuid,
                    "<BookmarkList></BookmarkList>",
                ),
                data("DC03$@CORP.EXAMPLE.COM", "nope", "<BookmarkList/>"),
            ],
        )
        .await?;

        assert_eq!(imported, 2);
        let mut exported = db.get_bookmarks(&uuid).await?;
        exported.sort_by(|a, b| a.machine.cmp(&b.machine));
        assert_eq!(
            exported,
            vec![
                data("DC01$@CORP.EXAMPLE.COM", &uuid, "<BookmarkList/>"),
                data(
                    "dc02.corp.example.com",
                    &uuid,
                    "<BookmarkList></BookmarkList>"
                ),
            ]
        );
        Ok(())
    }

    #[test]
    fn exported_file_is_what_import_reads_back() -> Result<()> {
        let path = tempfile::NamedTempFile::new()?.into_temp_path();
        let file = path.to_str().unwrap();
        let bookmarks = vec![
            data("DC01$@CORP.EXAMPLE.COM", "sub", "<BookmarkList/>"),
            data(
                "dc02.corp.example.com",
                "sub",
                "<BookmarkList></BookmarkList>",
            ),
        ];

        write_bookmarks(file, &bookmarks)?;

        assert_eq!(read_bookmarks(file)?, bookmarks);
        Ok(())
    }
}
