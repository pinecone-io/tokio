use std::time::Duration;
use crate::runtime::driver::{self, Driver};
use std::collections::VecDeque;
use crate::util::atomic_cell::AtomicCell;
use std::cell::RefCell;
use crate::runtime::task::{self, JoinHandle, OwnedTasks, Schedule, Task};
use crate::loom::sync::{Arc, Mutex};
use crate::util::{waker_ref, RngSeedGenerator, Wake, WakerRef};
use crate::runtime::{blocking, context, scheduler, Config};
use std::future::Future;
use std::task::Poll::{Pending, Ready};

use std::fmt;

/// Thread-local context.
struct Context {
    /// Scheduler handle
    handle: Arc<Handle>,
    core: RefCell<Option<Box<Core>>>,
}

fn did_defer_tasks() -> bool {
    context::with_defer(|deferred| !deferred.is_empty()).unwrap()
}

fn wake_deferred_tasks() {
    context::with_defer(|deferred| deferred.wake());
}

impl Context {
    fn run_task<R>(&self, mut core: Box<Core>, f: impl FnOnce() -> R) -> (Box<Core>, R) {
        self.enter(core, || f()) // TODO what is budget about?
    }

    fn enter<R>(&self, core: Box<Core>, f: impl FnOnce() -> R) -> (Box<Core>, R) {
        // Store the scheduler core in the thread-local context
        //
        // A drop-guard is employed at a higher level.
        *self.core.borrow_mut() = Some(core);

        // Execute the closure while tracking the execution budget
        let ret = f();

        // Take the scheduler core back
        let core = self.core.borrow_mut().take().expect("core missing");
        (core, ret)
    }

        /// Blocks the current thread until an event is received by the driver,
    /// including I/O events, timer events, ...
    fn park(&self, mut core: Box<Core>, handle: &Handle) -> Box<Core> {
        let mut driver = core.driver.take().expect("driver missing");

        if handle.tasks.lock().as_ref().map(|t| t.is_empty()).unwrap_or(true) {
            let (c, _) = self.enter(core, || {
                driver.park(&handle.driver);
                wake_deferred_tasks();
            });

            core = c;
        }

        core.driver = Some(driver);
        core
    }

    /// Checks the driver for new events without blocking the thread.
    fn park_yield(&self, mut core: Box<Core>, handle: &Handle) -> Box<Core> {
        let mut driver = core.driver.take().expect("driver missing");

        let (mut core, _) = self.enter(core, || {
            driver.park_timeout(&handle.driver, Duration::from_millis(0));
            wake_deferred_tasks();
        });

        core.driver = Some(driver);
        core
    }
}

// Tracks the current CurrentThread.
scoped_thread_local!(static CURRENT: Context);

pub(crate) struct Yolo {
    core: AtomicCell<Core>,
}

pub(crate) struct Handle {
    owned: OwnedTasks<Arc<Handle>>,

    tasks: Mutex<Option<VecDeque<task::Notified<Arc<Handle>>>>>,

    /// Resource driver handles
    pub(crate) driver: driver::Handle,

    /// Current random number generator seed
    pub(crate) seed_generator: RngSeedGenerator,
}

struct Core {
    driver: Option<Driver>,
}

impl Yolo {
    pub(crate) fn new(
	driver: Driver,
        driver_handle: driver::Handle,
	seed_generator: RngSeedGenerator,
    ) -> (Yolo, Arc<Handle>) {
	let core = AtomicCell::new(Some(Box::new(Core {driver: Some(driver)})));
	let handle = Handle{owned: OwnedTasks::new(),
			    tasks: Mutex::new(Some(VecDeque::with_capacity(10))),
			    driver: driver_handle,
			    seed_generator};
	let yolo = Yolo {
	    core,
	};
	(yolo, Arc::new(handle))
    }
    
    pub(crate) fn block_on<F: Future>(&self, handle: &scheduler::Handle, future: F) -> F::Output {
	pin!(future);
	let mut enter = crate::runtime::context::enter_runtime(handle, false);

	let handle = handle.as_yolo();
	loop {
            if let Some(core) = self.take_core(handle) {
		println!("IKDEBUG took current core");
                return core.block_on(future);
            } else {
		panic!("WTF, no core");
	    }
	}
    }

    fn take_core(&self, handle: &Arc<Handle>) -> Option<CoreGuard<'_>> {
        let core = self.core.take()?;

        Some(CoreGuard {
            context: Context {
                handle: handle.clone(),
                core: RefCell::new(Some(core)),
            },
            scheduler: self,
        })
    }

}

impl Handle {
    /// Spawns a future onto the `CurrentThread` scheduler
    pub(crate) fn spawn<F>(
        me: &Arc<Self>,
        future: F,
        id: crate::runtime::task::Id,
    ) -> JoinHandle<F::Output>
    where
        F: crate::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
	println!("SPAWNING! {id}");
        let (handle, notified) = me.owned.bind(future, me.clone(), id);

	if let Some(notified) = notified {
            me.schedule(notified);
        }

	
	handle
    }

    fn pop(&self) -> Option<task::Notified<Arc<Handle>>> {
        match self.tasks.lock().as_mut() {
            Some(queue) => queue.pop_front(),
            None => None,
        }
    }
}

impl fmt::Debug for Yolo {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
	fmt.debug_struct("yolo { ... }").finish()
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("yolo::Handle { ... }").finish()
    }
}

impl Wake for Handle {
    fn wake(arc_self: Arc<Self>) {
        Wake::wake_by_ref(&arc_self)
    }

    /// Wake by reference
    fn wake_by_ref(arc_self: &Arc<Self>) {
        //arc_self.shared.woken.store(true, Release);
        arc_self.driver.unpark();
    }
}

impl Schedule for Arc<Handle> {
    fn release(&self, task: &Task<Self>) -> Option<Task<Self>> {
	//	println!("IKDEBUG releasing {:?}", task.header());
	println!("IKDEBUG releasing {}",  task.id());
        self.owned.remove(task)
	    
    }

    fn schedule(&self, task: task::Notified<Self>) {
	//CURRENT.with(|maybe_cx| match maybe_cx {
        //    Some(cx) if Arc::ptr_eq(self, &cx.handle) => {
	//let mut core = cx.core.borrow_mut();
	println!("IKDEBUG scheduling {}",  task.id());
	let mut guard = self.tasks.lock();
	if let Some(tasks) = guard.as_mut() {
	    tasks.push_back(task);
	    drop(guard);
	}
	self.driver.unpark();

    //}
    //_ => {
    //panic!("No context, wtf");
      //      }
       // });
    }


	// TODO unpark driver
	//CURRENT.with(|maybe_cx| match maybe_cx {
	    //let mut guard
        //Some(cx) if Arc::ptr_eq(self, &cx.handle) => {
	//println!("Scheduled inside context");
	  //  },
	   // _ => {
	//	println!("scheduled outside context");
	//    }
	//);
}

impl Yolo {
}


/// Used to ensure we always place the `Core` value back into its slot in
/// `CurrentThread`, even if the future panics.
struct CoreGuard<'a> {
    context: Context,
    scheduler: &'a Yolo,
}

impl CoreGuard<'_> {
    #[track_caller]
    fn block_on<F: Future>(self, future: F) -> F::Output {
        let ret = self.enter(|mut core, context| {
            let waker = waker_ref(&context.handle); // IK this wakes the first time
            let mut cx = std::task::Context::from_waker(&waker);

            pin!(future);

	    println!("IKDEBUG start loop");
	    'outer: loop {
		let handle = &context.handle;
		println!("IKDEBUG looping once");

		while let Some(task) = context.handle.pop() {
		    let task = context.handle.owned.assert_owner(task);
		    let (c, _) = context.run_task(core, || {
			task.run();
		    });
		    core = c;
		}
		
		//context.
		let res = future.as_mut().poll(&mut cx);
		if let Ready(v) = res {
		    return (core, Some(v));
		}
		
		// Yield to the driver, this drives the timer and pulls any
                // pending I/O events.
                core = context.park_yield(core, handle);
	    }
	});

        match ret {
            Some(ret) => ret,
            None => {
                // `block_on` panicked.
                panic!("a spawned task panicked and the runtime is configured to shut down on unhandled panic");
            }
        }
    }

    /// Enters the scheduler context. This sets the queue and other necessary
    /// scheduler state in the thread-local.
    fn enter<F, R>(self, f: F) -> R
    where
        F: FnOnce(Box<Core>, &Context) -> (Box<Core>, R),
    {
        // Remove `core` from `context` to pass into the closure.
        let core = self.context.core.borrow_mut().take().expect("core missing");

        // Call the closure and place `core` back
        let (core, ret) = CURRENT.set(&self.context, || f(core, &self.context));

        *self.context.core.borrow_mut() = Some(core);

        ret
    }
}
