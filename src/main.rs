use std::collections::HashSet;
use std::error::Error;
use std::path::Path;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use chrono::{Local, Utc};

async fn get_docker_directories() -> Result<Vec<String>, Box<dyn Error>> {
    let cgroup_path = "/sys/fs/cgroup/system.slice/";
    let mut docker_list = Vec::new();

    let entries = fs::read_dir(cgroup_path).await?;
    tokio::pin!(entries); // Pin the iterator to enable async operations

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            if let Some(dir_name) = path.file_name().and_then(|s| s.to_str()) {
                if dir_name.starts_with("docker-") {
                    docker_list.push(dir_name.to_string());
                }
            }
        }
    }

    Ok(docker_list)
}

async fn get_whitelist(docker_list: &Vec<String>) -> Result<Vec<(String, HashSet<i32>)>, Box<dyn Error>> {
    let cgroup_path = "/sys/fs/cgroup/system.slice/";
    let mut whitelist = Vec::new();

    for docker_dir in docker_list {
        let procs_path = format!("{}/{}/cgroup.procs", cgroup_path, docker_dir);
        if Path::new(&procs_path).exists() {
            let procs_content = fs::read_to_string(&procs_path).await?;
            let procs: HashSet<i32> = procs_content.lines().filter_map(|s| s.parse().ok()).collect();
            whitelist.push((docker_dir.clone(), procs));
        }
    }

    Ok(whitelist)
}

async fn monitor_procs(docker_dir: String, initial_procs: HashSet<i32>) -> Result<(), Box<dyn Error>> {
    let cgroup_path = format!("/sys/fs/cgroup/system.slice/{}/cgroup.procs", docker_dir);
    let mut known_procs = initial_procs;

    loop {
        if Path::new(&cgroup_path).exists() {
            let procs_content = fs::read_to_string(&cgroup_path).await?;
            let current_procs: HashSet<i32> = procs_content.lines().filter_map(|s| s.parse().ok()).collect();

            for proc in &current_procs {
                if !known_procs.contains(proc) {
                    let detection_time = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                    let cleaned_docker_dir = docker_dir.replace("docker-", "").replace(".scope", "");
                    println!(
                        "[{}] \t New process detected - \t {} \t {}",
                        detection_time, cleaned_docker_dir, proc
                    );

                    // Stop the Docker container
                    let stop_start = Utc::now();
                    let output = Command::new("docker")
                        .arg("stop")
                        .arg(&cleaned_docker_dir)
                        .output()
                        .await?;

                    if !output.status.success() {
                        eprintln!("Failed to stop Docker container: {}", cleaned_docker_dir);
                    } else {
                        println!("Docker container stopped: {}", cleaned_docker_dir);

                        // Start the Docker container
                        let output = Command::new("docker")
                            .arg("start")
                            .arg(&cleaned_docker_dir)
                            .output()
                            .await?;

                        let stop_end = Utc::now();
                        let duration = stop_end - stop_start;
                        if !output.status.success() {
                            eprintln!("Failed to start Docker container: {}", cleaned_docker_dir);
                        } else {
                            println!("Docker container started: {}", cleaned_docker_dir);
                            println!("Time taken from stop to start: {} ms", duration.num_milliseconds());
                        }
                    }

                    known_procs.insert(*proc);
                }
            }
        }

        sleep(Duration::from_nanos(1)).await; // Monitoring interval set to 1 nanosecond
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Step 1: Retrieve docker directories
    let docker_list = get_docker_directories().await?;

    // Step 2: Get initial whitelist of processes
    let whitelist = get_whitelist(&docker_list).await?;

    // Step 3: Print the docker directories and the whitelist
    println!("Docker directories: {:?}", docker_list);
    println!("Whitelist: {:?}", whitelist);

    // Step 4: Monitor each docker directory in a separate task
    for (docker_dir, procs) in whitelist {
        tokio::spawn(async move {
            if let Err(e) = monitor_procs(docker_dir, procs).await {
                eprintln!("Error monitoring procs: {}", e);
            }
        });
    }

    // Keep the main function running indefinitely
    loop {
        sleep(Duration::from_secs(3600)).await;
    }
}
