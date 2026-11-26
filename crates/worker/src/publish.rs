//! Publishing jobs: open the pull request of a delivery, then watch it until it merges.

use crate::{Outcome, Services, payload_id, text};
use openjam_core::jobs::Job;
use openjam_publish::{
    delivery::{self, Step},
    github::App,
};

pub async fn handle(svc: &Services, job: &Job) -> Result<Outcome, String> {
    let id = payload_id(job, "deliveryId")?;
    let withdraw = job.payload["withdraw"].as_bool().unwrap_or(false);
    let app = match App::from_env() {
        Ok(app) => app,
        Err(e) => {
            delivery::fail(&svc.pool, id, &e.to_string())
                .await
                .map_err(text)?;
            return Ok(Outcome::Done);
        }
    };
    match delivery::run(&svc.pool, app.as_ref(), id, withdraw).await {
        Ok(Step::Done) => Ok(Outcome::Done),
        Ok(Step::RetryAt(at)) => Ok(Outcome::RetryAt(at)),
        Err(error) => {
            if job.attempts >= job.max_attempts {
                delivery::fail(&svc.pool, id, &error).await.map_err(text)?;
            }
            Err(error)
        }
    }
}
