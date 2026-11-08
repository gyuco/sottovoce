use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use sv_models::{ModelError, ModelFile, ModelKind, ModelSpec, ModelStatus, ModelStore};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// Serve `body` rispettando `Range: bytes=N-` (se `ranges`), e registra gli header ricevuti.
#[derive(Clone)]
struct RangeServer {
    body: Arc<Vec<u8>>,
    ranges: bool,
    seen: Arc<Mutex<Vec<Option<String>>>>,
}

impl Respond for RangeServer {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let range = req
            .headers
            .get("range")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        self.seen.lock().unwrap().push(range.clone());
        let from = range.filter(|_| self.ranges).and_then(|r| {
            r.strip_prefix("bytes=")?
                .strip_suffix('-')?
                .parse::<usize>()
                .ok()
        });
        match from {
            Some(n) if n < self.body.len() => {
                ResponseTemplate::new(206).set_body_bytes(self.body[n..].to_vec())
            }
            Some(_) => ResponseTemplate::new(416),
            None => ResponseTemplate::new(200).set_body_bytes(self.body.to_vec()),
        }
    }
}

struct Fixture {
    server: MockServer,
    spec: ModelSpec,
    bodies: Vec<Vec<u8>>,
    seen: Vec<Arc<Mutex<Vec<Option<String>>>>>,
    _dir: tempfile::TempDir,
    store: ModelStore,
}

async fn fixture(ranges: bool) -> Fixture {
    let server = MockServer::start().await;
    let bodies = vec![payload(300_000, 1), payload(1_234, 7)];
    let names = ["model.onnx", "tokens.txt"];
    let mut files = Vec::new();
    let mut seen = Vec::new();
    for (name, body) in names.iter().zip(&bodies) {
        let log = Arc::new(Mutex::new(Vec::new()));
        Mock::given(method("GET"))
            .and(path(format!("/m/{name}")))
            .respond_with(RangeServer {
                body: Arc::new(body.clone()),
                ranges,
                seen: log.clone(),
            })
            .mount(&server)
            .await;
        seen.push(log);
        files.push(ModelFile {
            name: (*name).to_owned(),
            url: format!("{}/m/{name}", server.uri()),
            size: body.len() as u64,
            sha256: sha(body),
        });
    }
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path());
    Fixture {
        server,
        spec: ModelSpec {
            id: "test-model".into(),
            kind: ModelKind::Stt,
            name: "Test".into(),
            files,
        },
        bodies,
        seen,
        store,
        _dir: dir,
    }
}

#[tokio::test]
async fn downloads_verifies_and_reports_progress() {
    let fx = fixture(true).await;
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Missing);
    assert!(matches!(
        fx.store.ready_dir(&fx.spec),
        Err(ModelError::NotReady { .. })
    ));

    let mut last = None;
    fx.store
        .download(&fx.spec, |p| last = Some(p), &AtomicBool::new(false))
        .await
        .unwrap();

    let last = last.unwrap();
    assert_eq!(last.downloaded, fx.spec.size());
    assert_eq!(last.total, fx.spec.size());
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Ready);
    let dir = fx.store.ready_dir(&fx.spec).unwrap();
    for (f, body) in fx.spec.files.iter().zip(&fx.bodies) {
        assert_eq!(&std::fs::read(dir.join(&f.name)).unwrap(), body);
        assert!(!dir.join(format!("{}.part", f.name)).exists());
    }
    assert!(fx.store.disk_usage(&fx.spec) >= fx.spec.size());
}

#[tokio::test]
async fn resumes_from_partial_file() {
    let fx = fixture(true).await;
    let dir = fx.store.model_dir(&fx.spec);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.onnx.part"), &fx.bodies[0][..100_000]).unwrap();
    assert_eq!(
        fx.store.status(&fx.spec),
        ModelStatus::Partial {
            downloaded: 100_000
        }
    );

    fx.store
        .download(&fx.spec, |_| {}, &AtomicBool::new(false))
        .await
        .unwrap();

    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Ready);
    assert_eq!(
        fx.seen[0].lock().unwrap().as_slice(),
        [Some("bytes=100000-".to_owned())]
    );
    assert_eq!(std::fs::read(dir.join("model.onnx")).unwrap(), fx.bodies[0]);
}

#[tokio::test]
async fn restarts_when_server_ignores_range() {
    let fx = fixture(false).await;
    let dir = fx.store.model_dir(&fx.spec);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.onnx.part"), &fx.bodies[0][..5_000]).unwrap();

    fx.store
        .download(&fx.spec, |_| {}, &AtomicBool::new(false))
        .await
        .unwrap();

    assert_eq!(std::fs::read(dir.join("model.onnx")).unwrap(), fx.bodies[0]);
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Ready);
}

#[tokio::test]
async fn checksum_mismatch_discards_file() {
    let mut fx = fixture(true).await;
    fx.spec.files[1].sha256 = "0".repeat(64);

    let err = fx
        .store
        .download(&fx.spec, |_| {}, &AtomicBool::new(false))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ModelError::Checksum { ref file } if file == "tokens.txt"),
        "{err}"
    );
    let dir = fx.store.model_dir(&fx.spec);
    assert!(!dir.join("tokens.txt").exists());
    assert!(!dir.join("tokens.txt.part").exists());
    assert_ne!(fx.store.status(&fx.spec), ModelStatus::Ready);
}

#[tokio::test]
async fn altered_file_is_corrupt_and_redownloaded() {
    let fx = fixture(true).await;
    let no_cancel = AtomicBool::new(false);
    fx.store
        .download(&fx.spec, |_| {}, &no_cancel)
        .await
        .unwrap();

    // Stessa dimensione, contenuto diverso.
    let target = fx.store.model_dir(&fx.spec).join("tokens.txt");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&target, payload(fx.bodies[1].len(), 99)).unwrap();
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Corrupt);
    assert_eq!(
        fx.store.verify(&fx.spec).await.unwrap(),
        ModelStatus::Corrupt
    );

    fx.store
        .download(&fx.spec, |_| {}, &no_cancel)
        .await
        .unwrap();
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Ready);
    assert_eq!(std::fs::read(&target).unwrap(), fx.bodies[1]);
    // Il primo file era integro: non riscaricato.
    assert_eq!(fx.seen[0].lock().unwrap().len(), 1);
}

#[tokio::test]
async fn verify_restores_lost_marker() {
    let fx = fixture(true).await;
    fx.store
        .download(&fx.spec, |_| {}, &AtomicBool::new(false))
        .await
        .unwrap();
    std::fs::remove_file(fx.store.model_dir(&fx.spec).join(".verified.json")).unwrap();
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Corrupt);
    assert_eq!(fx.store.verify(&fx.spec).await.unwrap(), ModelStatus::Ready);
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Ready);
}

#[tokio::test]
async fn cancel_keeps_partial_for_resume() {
    let fx = fixture(true).await;
    let err = fx
        .store
        .download(&fx.spec, |_| {}, &AtomicBool::new(true))
        .await
        .unwrap_err();
    assert!(matches!(err, ModelError::Cancelled), "{err}");
    assert_ne!(fx.store.status(&fx.spec), ModelStatus::Ready);

    fx.store
        .download(&fx.spec, |_| {}, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Ready);
}

#[tokio::test]
async fn http_error_and_delete() {
    let mut fx = fixture(true).await;
    fx.spec.files[0].url = format!("{}/missing", fx.server.uri());
    let err = fx
        .store
        .download(&fx.spec, |_| {}, &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert!(matches!(err, ModelError::Http { status: 404 }), "{err}");

    fx.store.delete(&fx.spec).await.unwrap();
    assert!(!fx.store.model_dir(&fx.spec).exists());
    assert_eq!(fx.store.status(&fx.spec), ModelStatus::Missing);
    fx.store.delete(&fx.spec).await.unwrap();
}
