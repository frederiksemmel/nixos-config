use super::index::{Index, Video};
use super::models::{Dismissal, Kind, SeriesPlace, Started, VideoFact, VideoPlace};
use std::path::PathBuf;
use toasty::sql::{query, statement};
use toasty::stmt::{Type, Value};

const TABLES: [&str; 5] = [
    "legacy_files",
    "legacy_file_places",
    "legacy_show_places",
    "legacy_watching",
    "legacy_dismissed",
];

fn fields(row: &Value) -> &[Value] {
    match row {
        Value::Record(record) => &record.fields,
        _ => &[],
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn float(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::F64(number) => Some(*number),
        Value::I64(number) => Some(*number as f64),
        _ => None,
    }
}

fn int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::I64(number) => Some(*number),
        _ => None,
    }
}

fn boolean(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(flag) => Some(*flag),
        Value::I64(number) => Some(*number != 0),
        _ => None,
    }
}

pub async fn convert(db: toasty::Db, library: PathBuf, index: Index) -> toasty::Result<usize> {
    let mut connection = db.connection().await?;
    let present = query("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'legacy_%'")
        .column_types([Type::I64])
        .exec(&mut connection)
        .await?;
    if present.first().map(fields).and_then(|row| int(row.first())) != Some(TABLES.len() as i64) {
        return Ok(0);
    }
    let video = |path: Option<String>| path.and_then(|path| index.video(&library.join(path)));
    let mut transaction = connection.transaction().await?;
    let mut moved = 0;

    let rows = query("SELECT path, duration, credits, subtitle_delay, embedded_subtitles FROM legacy_files")
        .column_types([Type::String, Type::F64, Type::F64, Type::F64, Type::I64])
        .exec(&mut transaction)
        .await?;
    for row in rows.iter().map(fields) {
        let Some(Video { kind, id }) = video(text(row.first())) else {
            continue;
        };
        VideoFact::upsert_by_kind_and_video_id(kind, id)
            .duration(float(row.get(1)))
            .credits(float(row.get(2)))
            .subtitle_delay(float(row.get(3)).unwrap_or_default())
            .embedded_subtitles(int(row.get(4)))
            .exec(&mut transaction)
            .await?;
        moved += 1;
    }

    let rows = query(
        "SELECT profile_id, path, stopped_at, last_played, finished, audio_track, subtitle_track, subtitles_shown
         FROM legacy_file_places",
    )
    .column_types([Type::I64, Type::String, Type::F64, Type::I64, Type::Bool, Type::I64, Type::I64, Type::Bool])
    .exec(&mut transaction)
    .await?;
    for row in rows.iter().map(fields) {
        let (Some(profile), Some(Video { kind, id })) = (int(row.first()), video(text(row.get(1)))) else {
            continue;
        };
        VideoPlace::upsert_by_profile_id_and_kind_and_video_id(profile, kind, id)
            .stopped_at(float(row.get(2)))
            .last_played(int(row.get(3)))
            .finished(boolean(row.get(4)).unwrap_or_default())
            .audio_track(int(row.get(5)))
            .subtitle_track(int(row.get(6)))
            .subtitles_shown(boolean(row.get(7)))
            .exec(&mut transaction)
            .await?;
        moved += 1;
    }

    let rows = query("SELECT profile_id, show_key, season, episode, finished FROM legacy_show_places")
        .column_types([Type::I64, Type::String, Type::I64, Type::I64, Type::Bool])
        .exec(&mut transaction)
        .await?;
    for row in rows.iter().map(fields) {
        let (Some(profile), Some(show)) = (int(row.first()), text(row.get(1)).and_then(|key| index.show(&key))) else {
            continue;
        };
        SeriesPlace::upsert_by_profile_id_and_show_id(profile, show)
            .season(int(row.get(2)).unwrap_or_default())
            .episode(int(row.get(3)).unwrap_or_default())
            .finished(boolean(row.get(4)).unwrap_or_default())
            .exec(&mut transaction)
            .await?;
        moved += 1;
    }

    let rows = query("SELECT profile_id, kind, title_key, last_played FROM legacy_watching")
        .column_types([Type::I64, Type::String, Type::String, Type::I64])
        .exec(&mut transaction)
        .await?;
    for row in rows.iter().map(fields) {
        let kind = match text(row.get(1)).as_deref() {
            Some("series") => crate::watching::Kind::Series,
            Some("movie") => crate::watching::Kind::Movie,
            _ => continue,
        };
        let (Some(profile), Some(title)) = (int(row.first()), text(row.get(2)).and_then(|key| index.title(kind, &key)))
        else {
            continue;
        };
        let stored = match kind {
            crate::watching::Kind::Series => Kind::Series,
            crate::watching::Kind::Movie => Kind::Movie,
        };
        Started::upsert_by_profile_id_and_kind_and_title_id(profile, stored, title)
            .last_played(int(row.get(3)).unwrap_or_default())
            .exec(&mut transaction)
            .await?;
        moved += 1;
    }

    let rows = query("SELECT profile_id, path, at FROM legacy_dismissed")
        .column_types([Type::I64, Type::String, Type::I64])
        .exec(&mut transaction)
        .await?;
    for row in rows.iter().map(fields) {
        let (Some(profile), Some(Video { kind, id })) = (int(row.first()), video(text(row.get(1)))) else {
            continue;
        };
        Dismissal::upsert_by_profile_id_and_kind_and_video_id(profile, kind, id)
            .at(int(row.get(2)).unwrap_or_default())
            .exec(&mut transaction)
            .await?;
        moved += 1;
    }

    for table in TABLES {
        statement(format!("DROP TABLE \"{table}\"")).exec(&mut transaction).await?;
    }
    transaction.commit().await?;
    Ok(moved)
}
