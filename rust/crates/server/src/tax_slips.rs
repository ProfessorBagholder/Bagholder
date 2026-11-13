//! The person's tax slips, read from Wealthsimple's Documents after the day's pull
//! when the list is due (brief 82, PR 1; `SPEC.md` tax-slips-1): the documents list,
//! and each slip not kept before read from its file, laid out by `pdftotext` page by
//! page, and kept in the book (`bagholder_wealthsimple::documents`). On the pull's
//! own thread and sign-in, one request after another, so no second client renews
//! the sign-in beside it.

use bagholder_book::Book;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::ConnectionId;
use bagholder_wealthsimple::client::Client;

use crate::app::{log, App};
use crate::broker_reads::session_file;

/// The documents list read where it is due (`documents::list_due`, its days in
/// `zone`), and each tax slip not kept before read and kept. A read that fails is
/// the log's, and the next day's pull reads the list again while it is due.
pub fn read(app: &App, book: &Book, conn: ConnectionId, now: Timestamp, zone: &TimeZone) {
    let day = |t: Timestamp| t.to_zoned(zone.clone()).date();
    match book.last_read(conn, bagholder_wealthsimple::documents::READ_PART) {
        Ok(last) if !bagholder_wealthsimple::documents::list_due(last.map(day), day(now)) => return,
        Ok(_) => {}
        Err(e) => return log(&format!("bagholder: when Wealthsimple's tax slips were last read could not be read: {e}")),
    }
    let mut client = Client::new(&app.net, session_file(app));
    match bagholder_wealthsimple::documents::keep_new(book, conn, &mut client, &bagholder_market::pdftext::layout_pages, now) {
        Ok(kept) => {
            for (slip, reading) in &kept.new {
                let what = format!("a {} for {} of account {}", slip.kind.name().to_uppercase(), slip.year, slip.account);
                match reading {
                    Ok(()) => log(&format!("bagholder: {what} read from Wealthsimple's documents")),
                    Err(why) => log(&format!("bagholder: {what} kept as it came and not read: {why}")),
                }
            }
            for (document, why) in &kept.unread {
                log(&format!("bagholder: a tax slip of Wealthsimple's ({document}) could not be read, and is read again with the next day's pull while the list is due: {why}"));
            }
        }
        Err(e) => log(&format!("bagholder: Wealthsimple's tax slips could not be read, and are read again with the next day's pull while the list is due: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use bagholder_core::Broker;

    use super::*;

    /// Wealthsimple made up: a list of one slip, its signed link, and a file at the
    /// link that is no PDF; every request kept with whether it carried the sign-in.
    struct Made(Arc<Mutex<Vec<(String, bool)>>>);

    impl bagholder_net::Transport for Made {
        fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
            let signed = ask.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("authorization") && *v == "Bearer a1");
            let body = ask.body.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            let op = ["FetchIdentityPaginatedDocuments", "DocumentSignedUrlCreate"].into_iter().find(|o| body.contains(&format!("\"operationName\":\"{o}\""))).unwrap_or("a file");
            self.0.lock().unwrap().push((op.to_string(), signed));
            let reply = match op {
                "FetchIdentityPaginatedDocuments" => r#"{"data":{"identity":{"id":"identity-1","documents":{"totalCount":1,"offset":0,"results":[{"id":"made-up-document-1","createdAt":"2026-02-27T10:00:00.000Z","availableAt":"2026-02-27","displayAt":null,"filename":null,"period":"2025-01-01","frequency":"year","type":"t5","downloadUrl":null,"uploaderName":null,"s3BucketName":null,"s3Key":"made-up-key-1","category":"tax","account":{"id":"made-up-account-1","type":"non_registered"},"documents":null}]}}}}"#,
                "DocumentSignedUrlCreate" => r#"{"data":{"signDocumentUrl":{"downloadUrl":"https://files.example.com/made-up-key-1?signature=made-up"}}}"#,
                _ => "a made-up page that is no PDF",
            };
            Ok((200, ask.url.to_string(), vec![], reply.as_bytes().to_vec()))
        }
    }

    /// The read after the day's pull asks with the app's own saved sign-in, the
    /// file without it; a file with no text is kept out and the read noted in the
    /// book, once a day, so the next day's pull asks for it again.
    #[test]
    fn the_slips_are_read_with_the_app_s_own_sign_in_and_the_read_noted() {
        let dir = tempfile::tempdir().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let net = bagholder_net::Net::answered_by(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()), Box::new(Made(asked.clone())));
        let app = App::with_net(dir.path().to_path_buf(), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net);
        let path = session_file(&app).path;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2999-01-01T00:00:00Z","wssdi":"device-1"}"#).unwrap();
        let now: Timestamp = "2026-03-02T22:00:00Z".parse().unwrap();
        let books = tempfile::tempdir().unwrap();
        let (book, _) = Book::open_in(books.path(), "test", now).unwrap();
        let conn = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        let zone = TimeZone::get("America/Edmonton").unwrap();
        read(&app, &book, conn, now, &zone);
        let first = vec![("FetchIdentityPaginatedDocuments".to_string(), true), ("DocumentSignedUrlCreate".to_string(), true), ("a file".to_string(), false)];
        assert_eq!(*asked.lock().unwrap(), first);
        assert!(book.tax_slip_documents(conn).unwrap().is_empty());
        assert_eq!(book.last_read(conn, bagholder_wealthsimple::documents::READ_PART).unwrap(), Some(now));
        // the same day again: the list is read once a day at most
        read(&app, &book, conn, "2026-03-02T23:30:00Z".parse().unwrap(), &zone);
        assert_eq!(*asked.lock().unwrap(), first);
        // the next day, still before its slips are all in: read again, the file again
        read(&app, &book, conn, "2026-03-03T22:00:00Z".parse().unwrap(), &zone);
        assert_eq!(asked.lock().unwrap().len(), 6);
    }
}
