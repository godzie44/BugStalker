use std::error::Error;
use std::time::Duration;
use std::vec;

#[tokio::main(worker_threads = 3)]
async fn main() -> Result<(), Box<dyn Error>> {
    let t1 = tokio::task::spawn(f1());
    t1.await.unwrap();

    let t2 = tokio::task::spawn(f2());
    t2.await.unwrap();

    Ok(())
}

async fn f1() {
    let mut vector: Vec<i32> = vec![1, 2, 3, 4, 5, 6, 7];
    let _a = 123;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _b = inner_1(&mut vector, 1).await;

    tokio::time::sleep(Duration::from_secs(2)).await;
    let _c = inner_1(&mut vector, 1).await;
}

async fn f2() {
    let mut vector: Vec<i32> = vec![1, 2, 3, 4, 5, 6, 7];
    let _a = 123;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _b = inner_1(&mut vector, 2).await;

    tokio::time::sleep(Duration::from_secs(2)).await;
    let _c = inner_1(&mut vector, 2).await;
}

async fn inner_1(v: &mut Vec<i32>, val: i32) -> i32 {
    v.push(val);
    let b = val;
    return b;
}
