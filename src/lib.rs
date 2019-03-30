#![feature(futures_api)]

use tokio_current_thread::CurrentThread;
use tokio_reactor::{self, Reactor};
use tokio_threadpool::{blocking, ThreadPool};
use tokio_timer::clock::{self, Clock};
use tokio_timer::timer::{self, Timer};

use std::cell::RefCell;
use std::time::Duration;

use futures::future::poll_fn;
use futures::sync::oneshot;
use futures::Future;

pub mod async_await;

thread_local!(static RT: RefCell<Runtime> = RefCell::new(Runtime::new()));
thread_local!(static POOL: RefCell<ThreadPool> = RefCell::new(ThreadPool::new()));

// re-exports
pub use tokio_current_thread::{spawn, RunError};
pub use crate::async_await::*;

/// Minimal signle-threaded tokio runtime that runs by calling [`turn`] or [`turn_with_timeout`].
///
/// It's useful when you work with external applications and they require to export some tick function.
///
/// [`turn`]: ./fn.turn.html
/// [`turn_with_timeout`]: ./fn.turn_with_timeout.html
pub struct Runtime {
    reactor_handle: tokio_reactor::Handle,
    timer_handle: timer::Handle,
    clock: Clock,
    executor: CurrentThread<Timer<Reactor>>,
}

impl Runtime {
    fn new() -> Runtime {
        let clock = Clock::system();
        let reactor = Reactor::new().expect("Couldn't create runtime");
        let reactor_handle = reactor.handle();

        let timer = Timer::new_with_now(reactor, clock.clone());
        let timer_handle = timer.handle();

        let executor = CurrentThread::new_with_park(timer);

        Runtime {
            reactor_handle,
            timer_handle,
            clock,
            executor,
        }
    }

    fn turn(&mut self, max_wait: Option<Duration>) -> bool {
        self.enter(|executor| executor.turn(max_wait))
            .map(|turn| turn.has_polled())
            .unwrap_or(false)
    }

    fn run(&mut self) -> Result<(), RunError> {
        self.enter(|executor| executor.run())
    }

    fn enter<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut tokio_current_thread::Entered<Timer<Reactor>>) -> R,
    {
        let Runtime {
            ref reactor_handle,
            ref timer_handle,
            ref clock,
            ref mut executor,
            ..
        } = *self;

        let mut enter = tokio_executor::enter().expect("multiple instances");

        tokio_reactor::with_default(&reactor_handle, &mut enter, |enter| {
            clock::with_default(&clock, enter, |enter| {
                timer::with_default(&timer_handle, enter, |enter| {
                    let mut default_executor = tokio_current_thread::TaskExecutor::current();
                    tokio_executor::with_default(&mut default_executor, enter, |enter| {
                        let mut exectutor = executor.enter(enter);
                        f(&mut exectutor)
                    })
                })
            })
        })
    }
}

/// Initialize a [`Runtime`] and spawn (like [`tokio::run`]) a future.
///
/// It does:
/// * Creates a thread local [`Runtime`] instance.
/// * Spawns the passed future into the executor of the [`Runtime`].
/// * Returns flow control back to you.
///
/// [`tokio::run`]: https://docs.rs/tokio/0.1/tokio/runtime/fn.run.html
/// [`Runtime`]: ./struct.Runtime.html
pub fn init_with<F>(future: F)
where
    F: Future<Item = (), Error = ()> + 'static,
{
    RT.with(|rt| {
        rt.borrow_mut().executor.spawn(future);
    });
}

/// Manually turns the [`Runtime`] with 100 ms timeout.
///
/// [`Runtime`]: ./struct.Runtime.html
pub fn turn() -> bool {
    RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.turn(Some(Duration::from_millis(100)))
    })
}

/// Same as [`turn`] but with explict timeout.
///
/// [`turn`]: ./fn.turn.html
pub fn turn_with_timeout(max_wait: Option<Duration>) -> bool {
    RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.turn(max_wait)
    })
}

/// Creates a thread local [`ThreadPool`] and spawns an expensive task on it.
///
/// [`ThreadPool`]: https://docs.rs/tokio-threadpool/0.1.13/tokio_threadpool/struct.ThreadPool.html
pub fn expensive_task<F, R>(mut task: F) -> impl Future<Item = R, Error = ()>
where
    F: FnMut() -> R + Send + Sync,
    F: 'static,
    R: Send + 'static,
{
    let (tx, rx) = oneshot::channel();

    POOL.with(move |thread_pool| {
        let task = poll_fn(move || blocking(|| task()))
            .map_err(|_| ())
            .map(|response| {
                let _ = tx.send(response);
            });

        thread_pool.borrow_mut().spawn(task);
    });

    rx.map_err(|_| ())
}

/// Run the executor to completion, blocking the thread until all spawned futures have completed.
pub fn run() -> Result<(), RunError> {
    RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.run()
    })
}
