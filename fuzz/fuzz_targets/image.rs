#![no_main]

use std::{
    cell::RefCell,
    io::{Seek, SeekFrom, Write},
};

use libfuzzer_sys::fuzz_target;
use tempfile::NamedTempFile;

thread_local! {
    static IMAGE: RefCell<NamedTempFile> = RefCell::new(
        NamedTempFile::new().expect("create reusable fuzz input file")
    );
}

fuzz_target!(
    init: {
        // Production catches unwind from the HFS+/APFS dependency boundary.
        // libfuzzer-sys normally aborts from its panic hook before that catch can
        // run, so replace only the hook. An uncontained panic still reaches
        // libfuzzer-sys's outer catch and aborts the fuzz process.
        std::panic::set_hook(Box::new(|_| {}));
    },
    |data: &[u8]| {
        IMAGE.with(|slot| {
            let mut image = slot.borrow_mut();
            let file = image.as_file_mut();
            file.set_len(0).expect("truncate fuzz input file");
            file.seek(SeekFrom::Start(0))
                .expect("rewind fuzz input file");
            file.write_all(data).expect("write fuzz input file");
            file.flush().expect("flush fuzz input file");
            pydmg::fuzzing::image(image.path());
        });
    }
);
