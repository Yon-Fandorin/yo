//! Preparation worker and execution-host evidence for later whole-input admission.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
};

use yo_core::{
    AgentCommand, ImagePreparationHost, ImagePreparationRequest, ImagePreparationSource,
    ImagePreparationUpdate, InputAdmissionHost, InputImage, InputImageDisplay,
    PreparedImageAttachment, ResolvedSkill, SubmissionRejection, SubmissionRejectionKind,
    TranscriptRecord, UserInput, session_repository::InheritedSessionHistory,
};

type Evidence = (String, u64, String);
type Registry = Arc<Mutex<HashSet<Evidence>>>;

pub(crate) fn bind(
    inner: Box<dyn InputAdmissionHost>,
    workspace: &Path,
    records: &[TranscriptRecord],
    inherited: Option<&InheritedSessionHistory>,
) -> (Box<dyn InputAdmissionHost>, Box<dyn ImagePreparationHost>) {
    let mut evidence = HashSet::new();
    let inherited_records = inherited
        .into_iter()
        .flat_map(|history| history.sections())
        .flat_map(|section| section.records());
    for record in records.iter().chain(inherited_records) {
        if let TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. },
        ) = record
        {
            for image in input.images() {
                evidence.insert(identity(image));
            }
        }
    }
    let registry = Arc::new(Mutex::new(evidence));
    (
        Box::new(Admission {
            inner,
            registry: registry.clone(),
        }),
        Box::new(Preparation {
            workspace: workspace.to_owned(),
            registry,
            job: None,
        }),
    )
}

fn identity(image: &InputImage) -> Evidence {
    (
        image.snapshot().sha256().to_owned(),
        image.source_byte_length(),
        serde_json::to_string(&image.display()).expect("bounded image display is serializable"),
    )
}

struct Admission {
    inner: Box<dyn InputAdmissionHost>,
    registry: Registry,
}

impl InputAdmissionHost for Admission {
    fn validate_images(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        let registry = self.registry.lock().map_err(|_| unavailable())?;
        if input
            .images()
            .iter()
            .any(|image| !registry.contains(&identity(image)))
        {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::Unauthorized,
                "Image source evidence was not prepared by this execution host. Attach the source again.",
            ));
        }
        Ok(())
    }
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        self.validate_images(input)?;
        self.inner.validate(input)
    }
    fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        self.validate_images(input)?;
        self.inner.prepare(input)
    }
}

struct Preparation {
    workspace: PathBuf,
    registry: Registry,
    job: Option<Job>,
}
struct Job {
    id: u64,
    revision: u64,
    cancel: Arc<AtomicBool>,
    wake: Arc<Mutex<Option<Waker>>>,
    receiver: mpsc::Receiver<ImagePreparationUpdate>,
    thread: JoinHandle<()>,
}

impl ImagePreparationHost for Preparation {
    fn start(&mut self, request: ImagePreparationRequest) -> Result<(), SubmissionRejection> {
        if self.job.is_some() {
            return Err(SubmissionRejection::new(
                SubmissionRejectionKind::OverBudget,
                "One image is still being prepared. Wait for it to finish before attaching another.",
            ));
        }
        let source = match request.source {
            ImagePreparationSource::File(path) => {
                ImagePreparationSource::File(self.workspace.join(path))
            },
            ImagePreparationSource::Clipboard => ImagePreparationSource::Clipboard,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Mutex::new(None::<Waker>));
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_cancel = cancel.clone();
        let worker_wake = wake.clone();
        let registry = self.registry.clone();
        let id = request.id;
        let revision = request.revision;
        let thread = thread::Builder::new()
            .name("yo-image-preparation".to_owned())
            .spawn(move || {
                let result = prepare_attachment(&source, &worker_cancel).and_then(|prepared| {
                    if worker_cancel.load(Ordering::Acquire) {
                        return Err(cancelled());
                    }
                    registry
                        .lock()
                        .map_err(|_| unavailable())?
                        .insert(identity(prepared.image()));
                    Ok(prepared)
                });
                let _ = sender.send(ImagePreparationUpdate {
                    id,
                    revision,
                    result,
                });
                if let Ok(mut wake) = worker_wake.lock()
                    && let Some(wake) = wake.take()
                {
                    wake.wake();
                }
            })
            .map_err(|_| unavailable())?;
        self.job = Some(Job {
            id,
            revision,
            cancel,
            wake,
            receiver,
            thread,
        });
        Ok(())
    }
    fn poll(&mut self, context: &mut Context<'_>) -> Poll<ImagePreparationUpdate> {
        let Some(job) = self.job.as_ref() else {
            return Poll::Pending;
        };
        if let Ok(mut wake) = job.wake.lock() {
            *wake = Some(context.waker().clone());
        }
        let update = match job.receiver.try_recv() {
            Ok(update) => update,
            Err(mpsc::TryRecvError::Empty) => return Poll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => ImagePreparationUpdate {
                id: job.id,
                revision: job.revision,
                result: Err(unavailable()),
            },
        };
        if let Some(job) = self.job.take() {
            let _ = job.thread.join();
        }
        Poll::Ready(update)
    }
    fn cancel(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Release);
        }
    }
}

impl Drop for Preparation {
    fn drop(&mut self) {
        self.cancel();
        if let Some(job) = self.job.take() {
            let _ = job.thread.join();
        }
    }
}

fn prepare_attachment(
    source: &ImagePreparationSource,
    cancel: &AtomicBool,
) -> Result<PreparedImageAttachment, SubmissionRejection> {
    let mut is_cancelled = || cancel.load(Ordering::Acquire);
    let result = match source {
        ImagePreparationSource::File(path) => super::prepare(path, &mut is_cancelled),
        ImagePreparationSource::Clipboard => {
            #[cfg(unix)]
            {
                super::clipboard::prepare(&mut is_cancelled)
            }
            #[cfg(not(unix))]
            {
                Err(crate::interaction::diagnostic::AppError::message(
                    "Clipboard images are unavailable on this platform. Use /attach with a PNG or JPEG path.",
                ))
            }
        },
    };
    let prepared = result.map_err(|error| {
        if cancel.load(Ordering::Acquire) {
            cancelled()
        } else {
            SubmissionRejection::new(
                SubmissionRejectionKind::InvalidReference,
                format!("{error} Use /attach with a static PNG or JPEG path if needed."),
            )
        }
    })?;
    let filename = match source {
        ImagePreparationSource::File(path) => path.file_name(),
        ImagePreparationSource::Clipboard => None,
    }
    .and_then(|name| name.to_str())
    .filter(|name| {
        !name.is_empty()
            && name.len() <= 255
            && !name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
    })
    .map(str::to_owned);
    let image = InputImage::new(
        0..InputImage::PROJECTION.len(),
        prepared.source_byte_length(),
        prepared.snapshot().clone(),
    )
    .and_then(|image| {
        image.with_display(InputImageDisplay {
            filename,
            source_mime_type: Some(prepared.source_mime_type().to_owned()),
            source_width: Some(prepared.source_width()),
            source_height: Some(prepared.source_height()),
            source_sha256: Some(prepared.source_sha256().to_owned()),
        })
    })
    .map_err(|_| unavailable())?;
    let png = prepared.thumbnail_png();
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    PreparedImageAttachment::new(image, png.to_vec(), width, height)
}

fn unavailable() -> SubmissionRejection {
    SubmissionRejection::new(
        SubmissionRejectionKind::EnvironmentUnavailable,
        "Image preparation is unavailable.",
    )
}
fn cancelled() -> SubmissionRejection {
    SubmissionRejection::new(
        SubmissionRejectionKind::TargetChanged,
        "Image preparation was cancelled.",
    )
}

#[cfg(test)]
mod tests;
