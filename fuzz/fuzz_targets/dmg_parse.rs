#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    pydmg::fuzzing::dmg_parse(data);
});
