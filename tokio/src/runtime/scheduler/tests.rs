use crate::sync::mpsc;

#[derive(Debug,PartialEq)]
struct Invokation {
    task: String,
    iteration: usize,
}

#[test]
fn test_invokation_order() {
    let run1 = run_with_seed(1);
    let run2 = run_with_seed(1);
    let run3 = run_with_seed(2);

    assert_eq!(run1, run2);
    assert_ne!(run1, run3);
}

fn run_with_seed(seed: u64) -> Vec<Invokation> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Invokation>();
    
    let runtime = crate::runtime::Builder::new_yolo().enable_all()
	.yolo_settings(crate::runtime::scheduler::yolo::YoloSettings{seed}).build().expect("blah");
    //let runtime = crate::runtime::Builder::new_current_thread().build().expect("blah");
    let f1tx = tx.clone();
    let f1 = runtime.spawn(async move {
	f1tx.send(Invokation{task: "f1".to_string(), iteration: 0}).unwrap();
    });
    let f2tx = tx.clone();
    let f2 = runtime.spawn(async move {
	f2tx.send(Invokation{task: "f2".to_string(), iteration: 0}).unwrap();
    });
    let f3tx = tx.clone();
    let f3 = runtime.spawn(async move {
	for i in 0..10 {
	    crate::time::sleep(crate::time::Duration::from_secs(3)).await;
	    f3tx.send(Invokation{task: "f3".to_string(), iteration: i}).unwrap();
	}
    });
    let f4tx = tx.clone();
    let f4 = runtime.spawn(async move {
	for i in 0..20 {
	    crate::time::sleep(crate::time::Duration::from_secs(1)).await;
	    f4tx.send(Invokation{task: "f4".to_string(), iteration: i}).unwrap();
	    
	    if i == 3 {
		let f5tx = f4tx.clone();
		crate::spawn(async move {
		    for i in 0..3 {
			crate::time::sleep(crate::time::Duration::from_secs(5)).await;
			f5tx.send(Invokation{task: "f5".to_string(), iteration: i}).unwrap();
		    }
		});
	    }
	}
    });
    
    runtime.block_on(async move {
	async {
	    _ = f1.await;
	    _ = f2.await;
	    _ = f3.await;
	    _ = f4.await;
	}.await;
    });

    let mut order = Vec::new();
    while let Ok(invokation) = rx.try_recv() {
	order.push(invokation);
    }
    order
}

#[test]
fn test_demo() {
    demo_run_with_seed(false, 1);
    demo_run_with_seed(true, 1);
    demo_run_with_seed(true, 1);
    demo_run_with_seed(true, 2);
    demo_run_with_seed(true, 1);
}

fn demo_run_with_seed(yolo: bool, seed: u64) {
    println!("Demo run, seed {seed}, using yolo {yolo}");
    let runtime = if yolo {
	crate::runtime::Builder::new_yolo().enable_all()
	    .yolo_settings(crate::runtime::scheduler::yolo::YoloSettings{seed}).build().expect("blah")
    } else {
	crate::runtime::Builder::new_current_thread().enable_all().build().expect("blah")
    };

    let t1 = runtime.spawn(async move {
	for i in 0..5 {
	    crate::time::sleep(crate::time::Duration::from_millis(100)).await;
	    println!("task 1, iteration {i} - {}", 10 + i);
	}
    });
    let t2 = runtime.spawn(async move {
	for i in 0..10 {
	    crate::time::sleep(crate::time::Duration::from_millis(20)).await;
	    println!("task 2, iteration {i} - {}", 20 + i);
	    
	    if i == 3 {
		crate::spawn(async move {
		    for i in 0..3 {
			crate::time::sleep(crate::time::Duration::from_millis(200)).await;
			println!("task 3, iteration {i} - {}", 30 + i);
		    }
		});
	    }
	}
    });
    
    runtime.block_on(async move {
	async {
	    _ = t1.await;
	    _ = t2.await;
	}.await;
    });
}
