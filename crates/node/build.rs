//! napi-rs codegen hook. Reads every `#[napi]` item in `src/` and
//! emits the TypeScript declarations + JS glue so `npm install` users
//! get proper `.d.ts` for autocomplete.

fn main() {
    napi_build::setup();
}
