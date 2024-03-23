use std::time::Duration;

async fn new_ticker_task(task_name: &str, seed: u64) {
    loop {
        tokio::time::sleep(Duration::from_secs(seed)).await;
        println!("task \"{task_name}\" tick!");
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .unwrap();

    runtime.spawn(new_ticker_task("task_1", 1));
    runtime.spawn(new_ticker_task("task_2", 2));
    runtime.spawn(new_ticker_task("task_3", 3));

    std::thread::sleep(Duration::from_secs(3));
    runtime.spawn(new_ticker_task("task_4", 2));
    std::thread::sleep(Duration::from_secs(3));
    runtime.spawn(new_ticker_task("task_5", 2));
    std::thread::sleep(Duration::from_secs(3));
    runtime.spawn(new_ticker_task("task_6", 2));
    std::thread::sleep(Duration::from_secs(3));
    runtime.spawn(new_ticker_task("task_7", 2));
    std::thread::sleep(Duration::from_secs(3));

    drop(runtime);

    std::thread::sleep(Duration::from_secs(1));
}
