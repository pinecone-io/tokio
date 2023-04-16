#[test]
fn hello_world() {
    let runtime = crate::runtime::Builder::new_yolo().enable_all().build().expect("blah");
    //let runtime = crate::runtime::Builder::new_current_thread().build().expect("blah");
    let f = runtime.spawn(async {
	println!("blah");
    });
    let f2 = runtime.spawn(async {
	println!("blab");
    });
    let f3 = runtime.spawn(async {
	println!("wee");
    });
    let f4 = runtime.spawn(async {
	crate::time::sleep(crate::time::Duration::from_secs(3)).await;
	println!("ffff");
    });
    
    // I need to get more than one waiting at a time
    println!("IKDEBUG block on");
    runtime.block_on(async move {
	async {
	    _ = f.await;
	    _ = f2.await;
	    _ = f3.await;
	    _ = f4.await;
	}.await;
	println!("foobar");
    });

}
