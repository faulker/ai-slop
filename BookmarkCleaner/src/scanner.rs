use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc;
use crate::parser::Bookmark;

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub enum LinkStatus {
    Ok,
    Dead(String), // Reason
    Upgraded(String), // New URL
}

pub async fn scan_bookmarks(
    bookmarks: Vec<Bookmark>, 
    tx: mpsc::Sender<(usize, LinkStatus)>, 
    redirect_limit: usize, 
    ignore_ssl: bool, 
    concurrent_requests: usize,
    timeout_secs: u64,
    retries: u32
) {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent(USER_AGENT)
        .danger_accept_invalid_certs(ignore_ssl)
        .redirect(reqwest::redirect::Policy::limited(redirect_limit))
        .build()
        .unwrap_or_default();

    // Semaphore to limit concurrency
    let max_concurrent = if concurrent_requests == 0 { 1 } else { concurrent_requests };
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    
    let mut handles = Vec::new();

    for (index, bookmark) in bookmarks.into_iter().enumerate() {
        let client = client.clone();
        let tx = tx.clone();
        let permit = semaphore.clone().acquire_owned().await.unwrap();

        let handle = tokio::spawn(async move {
            let status = check_link_smart(&client, &bookmark.url, retries).await;
            let _ = tx.send((index, status)).await;
            drop(permit);
        });
        handles.push(handle);
    }
    
    // Wait for all to finish (or just let them run, but we need to drop tx to close channel)
    // Actually, we can just await the join handles if we want to ensure everything is done
    // But main loop is receiving. 
    // Best pattern: spawn a task that awaits all handles and then exits?
    // Or just let the handles run detached?
    // We want to know when we are "done".
    
    for h in handles {
        let _ = h.await;
    }
    
    // Explicitly drop original tx so the receiver loop knows we are done
    drop(tx);
}

async fn check_link_smart(client: &Client, url: &str, retries: u32) -> LinkStatus {
    // 1. Check original URL
    let status = check_link(client, url, retries).await;
    
    // 2. If Dead and HTTP, try HTTPS
    if let LinkStatus::Dead(_) = status {
        if url.starts_with("http://") {
            let https_url = url.replace("http://", "https://");
            let https_status = check_link(client, &https_url, retries).await;
            
            if let LinkStatus::Ok = https_status {
                return LinkStatus::Upgraded(https_url);
            }
        }
    }
    
    status
}

async fn check_link(client: &Client, url: &str, max_retries: u32) -> LinkStatus {
    // Basic validation first
    if !url.starts_with("http") {
        return LinkStatus::Ok; // Skip non-http links (javascript:, file:, etc)
    }

    let mut attempts = 0;

    loop {
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return LinkStatus::Ok;
                } else if status.as_u16() == 404 || status.as_u16() == 410 {
                    return LinkStatus::Dead(format!("{} Not Found/Gone", status));
                } else {
                    // Treat all other status codes (403, 500, 503, 429, etc.) as potentially alive.
                    // We don't want to delete bookmarks just because of temporary server issues or blocking.
                    return LinkStatus::Ok;
                }
            },
            Err(e) => {
                if attempts >= max_retries {
                    if e.is_timeout() {
                        return LinkStatus::Dead("Timeout".to_string());
                    } else if e.is_connect() {
                         return LinkStatus::Dead("DNS/Connection Error".to_string());
                    } else if e.is_redirect() {
                         return LinkStatus::Dead("Redirect Loop".to_string());
                    } else {
                         return LinkStatus::Dead(e.to_string());
                    }
                }
                
                // Only retry on timeout or connection errors
                if e.is_timeout() || e.is_connect() {
                    attempts += 1;
                    // Small backoff could be useful
                    tokio::time::sleep(Duration::from_millis(2000)).await;
                    continue;
                }
                
                // Other errors (redirect loop, url parse error) are fatal
                return LinkStatus::Dead(e.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flatuicolors() {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap();

        for i in 0..10 {
            let status = check_link(&client, "http://flatuicolors.com/", 3).await;
            println!("Attempt {}: {:?}", i, status);
            if let LinkStatus::Dead(reason) = &status {
                panic!("Link reported dead on attempt {}: {}", i, reason);
            }
        }
    }

    #[tokio::test]
    async fn test_logobook() {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap();

        let status = check_link(&client, "https://logobook.com/", 3).await;
        println!("Logobook Status: {:?}", status);
        if let LinkStatus::Dead(reason) = &status {
            panic!("Logobook reported dead: {}", reason);
        }
    }
}