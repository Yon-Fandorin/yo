use std::{
    fs,
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};

use super::*;

struct SkillSpy(Arc<AtomicUsize>);
impl InputAdmissionHost for SkillSpy {
    fn validate(&self, _: &UserInput) -> Result<(), SubmissionRejection> {
        Ok(())
    }
    fn prepare(&self, _: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yo-image-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn poll(host: &mut dyn ImagePreparationHost) -> ImagePreparationUpdate {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(update) = host.poll(&mut context) {
            return update;
        }
        assert!(Instant::now() < deadline, "bounded image worker completion");
        thread::park_timeout(Duration::from_millis(10));
    }
}

// 공개 snapshot 구성은 준비 근거가 아니며 실제 작업 결과만 skill 로딩 전 검증을 통과한다.
#[test]
fn worker_evidence_is_required_before_skill_preparation() {
    let directory = Directory::new();
    let png = super::super::encode(&image::RgbaImage::new(1, 1), 1024).unwrap();
    fs::write(directory.0.join("source.png"), &png).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (admission, mut host) = bind(Box::new(SkillSpy(calls.clone())), &directory.0, &[], None);
    let snapshot = yo_core::InputImageSnapshot::new(1, 1, png).unwrap();
    let invented = UserInput::from("[image]")
        .with_images(vec![InputImage::new(0..7, 1, snapshot).unwrap()])
        .unwrap();
    assert!(admission.prepare(&invented).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let request = ImagePreparationRequest {
        id: 7,
        revision: 11,
        source: ImagePreparationSource::File(PathBuf::from("source.png")),
    };
    host.start(request.clone()).unwrap();
    assert!(host.start(request).is_err());
    let update = poll(host.as_mut());
    assert_eq!((update.id, update.revision), (7, 11));
    let ready = update.result.unwrap();
    let input = UserInput::from("[image]")
        .with_images(vec![ready.image().clone()])
        .unwrap();
    admission.prepare(&input).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let altered = InputImage::new(
        0..7,
        ready.image().source_byte_length() + 1,
        ready.image().snapshot().clone(),
    )
    .unwrap()
    .with_display(ready.image().display().unwrap().clone())
    .unwrap();
    assert!(
        admission
            .validate_images(
                &UserInput::from("[image]")
                    .with_images(vec![altered])
                    .unwrap()
            )
            .is_err()
    );
}

// 취소한 작업은 ready 결과를 반환하지 않고 완료 후 다음 작업을 허용한다.
#[test]
fn cancellation_releases_the_single_preparation_lane() {
    let directory = Directory::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let (_, mut host) = bind(Box::new(SkillSpy(calls)), &directory.0, &[], None);
    host.start(ImagePreparationRequest {
        id: 1,
        revision: 2,
        source: ImagePreparationSource::File(PathBuf::from("missing.png")),
    })
    .unwrap();
    host.cancel();
    assert!(poll(host.as_mut()).result.is_err());
    host.start(ImagePreparationRequest {
        id: 2,
        revision: 3,
        source: ImagePreparationSource::File(PathBuf::from("missing.png")),
    })
    .unwrap();
    assert_eq!(poll(host.as_mut()).id, 2);
}
