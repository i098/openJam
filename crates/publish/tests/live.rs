//! The real GitHub API, against a repository you own. Not run by default:
//! `OPENJAM_LIVE_REPO=owner/name OPENJAM_GITHUB_APP_ID=... OPENJAM_GITHUB_APP_PRIVATE_KEY_FILE=...
//!  cargo test -p openjam-publish --test live -- --ignored --nocapture`
//! The App must be installed on that repository. The test opens a pull request and closes it.

use openjam_publish::github::App;

#[tokio::test]
#[ignore = "needs a GitHub App and a throwaway repository"]
async fn open_and_withdraw_a_pull_request() {
    let target = std::env::var("OPENJAM_LIVE_REPO").expect("OPENJAM_LIVE_REPO=owner/name");
    let (owner, name) = target.split_once('/').expect("owner/name");
    let app = App::from_env().unwrap().expect("OPENJAM_GITHUB_APP_ID");

    let installation = app
        .installation(owner, name)
        .await
        .unwrap()
        .expect("installed");
    assert!(
        installation.can_open_prs,
        "the App needs Contents and Pull requests: write"
    );
    let repo = app.repo(installation.id, owner, name).await.unwrap();
    let sha = repo.branch_sha("main").await.unwrap().expect("main exists");

    let branch = format!("jam-geo/live-test-{}", chrono::Utc::now().timestamp());
    repo.ensure_branch(&branch, &sha).await.unwrap();
    repo.ensure_branch(&branch, &sha).await.unwrap();
    repo.put_file(
        &branch,
        "content/blog/live-test.md",
        "---\ntitle: Live\n---\n\nHello\n",
        "Add live test",
    )
    .await
    .unwrap();
    assert!(
        repo.list_dir(&branch, "content/blog")
            .await
            .unwrap()
            .contains(&"live-test.md".to_owned())
    );
    let pr = repo
        .open_pr(&branch, "main", "Add article: Live", "openJam live test")
        .await
        .unwrap();
    println!("opened {}", pr.url);
    assert!(pr.open && !pr.merged);
    let again = repo
        .open_pr(&branch, "main", "Add article: Live", "openJam live test")
        .await
        .unwrap();
    assert_eq!(
        again.number, pr.number,
        "a retry returns the open pull request"
    );

    repo.withdraw(pr.number, &branch).await.unwrap();
    let closed = repo.pr(pr.number).await.unwrap();
    assert!(!closed.open && !closed.merged);
    assert!(
        repo.branch_sha(&branch).await.unwrap().is_none(),
        "branch deleted"
    );
}
