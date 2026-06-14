use once_cell::sync::Lazy;

pub static SERVER_RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| {
    let worker_threads = std::env::var("PROBING_SERVER_WORKER_THREADS")
        .unwrap_or_else(|_| "4".to_string())
        .parse::<usize>()
        .unwrap_or(4);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .thread_name("server runtime")
        .on_thread_start(|| {
            log::debug!(
                "start server runtime thread: {:?}",
                std::thread::current().id()
            );
        })
        .build()
        .unwrap_or_else(|e| panic!("Failed to create server runtime: {e}"))
});
