use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use serde::Serialize;

use crate::config::devspace_dir;

pub struct Db(pub Mutex<Connection>);

#[derive(Serialize)]
pub struct CleanedEntry {
    pub ts: u64,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Serialize)]
pub struct Forecast {
    /// Days until free disk hits the configured threshold; None if the trend
    /// is flat/positive or there isn't enough data yet.
    pub days_left: Option<f64>,
    pub samples: usize,
    pub trend_gb_per_day: f64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn open() -> Db {
    let conn = Connection::open(devspace_dir().join("history.db")).expect("open history.db");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS disk_history (ts INTEGER NOT NULL, free_bytes INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS cleaned (ts INTEGER NOT NULL, path TEXT NOT NULL, size_bytes INTEGER NOT NULL);",
    )
    .expect("create tables");
    Db(Mutex::new(conn))
}

impl Db {
    pub fn log_disk_free(&self, free_bytes: u64) {
        let conn = self.0.lock().unwrap();
        // At most one sample per hour.
        let last: Option<u64> = conn
            .query_row("SELECT MAX(ts) FROM disk_history", [], |r| r.get(0))
            .ok()
            .flatten();
        if last.is_none_or(|t| now() - t >= 3600) {
            let _ = conn.execute(
                "INSERT INTO disk_history (ts, free_bytes) VALUES (?1, ?2)",
                (now(), free_bytes),
            );
        }
    }

    pub fn log_cleaned(&self, path: &str, size_bytes: u64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO cleaned (ts, path, size_bytes) VALUES (?1, ?2, ?3)",
            (now(), path, size_bytes),
        );
    }

    pub fn recently_cleaned(&self) -> Vec<CleanedEntry> {
        let conn = self.0.lock().unwrap();
        let cutoff = now() - 86_400;
        let mut stmt = conn
            .prepare("SELECT ts, path, size_bytes FROM cleaned WHERE ts >= ?1 ORDER BY ts DESC")
            .unwrap();
        stmt.query_map([cutoff], |r| {
            Ok(CleanedEntry {
                ts: r.get(0)?,
                path: r.get(1)?,
                size_bytes: r.get(2)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    pub fn total_saved(&self) -> u64 {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT COALESCE(SUM(size_bytes), 0) FROM cleaned", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Linear regression over the last 30 days of hourly samples, weighted
    /// toward recent points (last 7 days count double).
    pub fn forecast(&self, current_free: u64, threshold_gb: f64) -> Forecast {
        const GB: f64 = 1024.0 * 1024.0 * 1024.0;
        let conn = self.0.lock().unwrap();
        let cutoff = now().saturating_sub(30 * 86_400);
        let mut stmt = conn
            .prepare("SELECT ts, free_bytes FROM disk_history WHERE ts >= ?1 ORDER BY ts")
            .unwrap();
        let mut points: Vec<(f64, f64)> = stmt
            .query_map([cutoff], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)))
            .map(|rows| {
                rows.filter_map(|r| r.ok())
                    .map(|(ts, free)| (ts as f64 / 86_400.0, free as f64 / GB))
                    .collect()
            })
            .unwrap_or_default();
        points.push((now() as f64 / 86_400.0, current_free as f64 / GB));

        if points.len() < 24 {
            return Forecast { days_left: None, samples: points.len(), trend_gb_per_day: 0.0 };
        }

        let now_d = now() as f64 / 86_400.0;
        let (mut sw, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(x, y) in &points {
            let w = if now_d - x <= 7.0 { 2.0 } else { 1.0 };
            sw += w;
            sx += w * x;
            sy += w * y;
            sxx += w * x * x;
            sxy += w * x * y;
        }
        let denom = sw * sxx - sx * sx;
        if denom.abs() < f64::EPSILON {
            return Forecast { days_left: None, samples: points.len(), trend_gb_per_day: 0.0 };
        }
        let slope = (sw * sxy - sx * sy) / denom; // GB per day
        let free_gb = current_free as f64 / GB;
        let days_left = if slope < -0.01 && free_gb > threshold_gb {
            Some((free_gb - threshold_gb) / -slope)
        } else {
            None
        };
        Forecast { days_left, samples: points.len(), trend_gb_per_day: slope }
    }
}
