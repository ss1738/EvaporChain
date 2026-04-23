#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        if let Ok(contract) = evaporchain_script::parser::parse(source) {
            let _ = evaporchain_script::compiler::compile(&contract);
        }
    }
});
