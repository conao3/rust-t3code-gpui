use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use wasmtime::{
    Caller, Config, Engine, Func, Instance, Linker, Memory, Module, OptLevel, Ref, Store,
    StoreLimits, StoreLimitsBuilder, TypedFunc, Val, WasmParams, WasmResults,
};

const WASM: &[u8] = include_bytes!("../assets/ghostty-vt.wasm");
static MODULE: OnceLock<std::result::Result<Arc<Module>, String>> = OnceLock::new();

pub fn checked<T>(result: std::result::Result<T, wasmtime::Error>) -> Result<T> {
    result.map_err(|error| anyhow::anyhow!("{error:#}"))
}

#[derive(Clone, Deserialize)]
pub struct Field {
    pub offset: usize,
    pub size: usize,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Deserialize)]
pub struct Layout {
    pub size: usize,
    pub fields: HashMap<String, Field>,
}

pub struct Host {
    pub memory: Option<Memory>,
    pub replies: Vec<Vec<u8>>,
    pub suppress_replies: bool,
    limits: StoreLimits,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            memory: None,
            replies: Vec::new(),
            suppress_replies: false,
            limits: StoreLimitsBuilder::new().memory_size(256 << 20).build(),
        }
    }
}

pub struct Runtime {
    pub store: Store<Host>,
    pub memory: Memory,
    instance: Instance,
    exports: HashMap<String, Func>,
    pub layouts: HashMap<String, Layout>,
    pub callback_index: i32,
}

impl Runtime {
    pub fn new() -> Result<Self> {
        let module = MODULE.get_or_init(|| {
            let mut config = Config::new();
            config.cranelift_opt_level(OptLevel::Speed);
            let result: Result<_> = (|| {
                let engine = checked(Engine::new(&config))?;
                let workers =
                    std::thread::available_parallelism().map_or(1, |count| count.get().min(4));
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .build()?;
                checked(pool.install(|| Module::new(&engine, WASM)))
            })();
            result.map(Arc::new).map_err(|error| format!("{error:#}"))
        });
        let module = module
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))?;
        let mut linker = Linker::new(module.engine());
        checked(linker.func_wrap("env", "log", |_: i32, _: i32| {}))?;
        let mut store = Store::new(module.engine(), Host::default());
        store.limiter(|host| &mut host.limits);
        let instance = checked(linker.instantiate(&mut store, module))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("Ghostty memory export")?;
        store.data_mut().memory = Some(memory);
        let callback = Func::wrap(
            &mut store,
            |mut caller: Caller<'_, Host>, _: i32, _: i32, pointer: i32, length: i32| {
                if caller.data().suppress_replies || length <= 0 || pointer < 0 {
                    return;
                }
                let Some(memory) = caller.data().memory else {
                    return;
                };
                if let Some(bytes) = memory
                    .data(&caller)
                    .get(pointer as usize..pointer as usize + length as usize)
                {
                    let bytes = bytes.to_vec();
                    caller.data_mut().replies.push(bytes);
                }
            },
        );
        let table = instance
            .get_table(&mut store, "__indirect_function_table")
            .context("Ghostty callback table")?;
        let index = checked(table.grow(&mut store, 1, Ref::Func(None)))?;
        checked(table.set(&mut store, index, Ref::Func(Some(callback))))?;
        let mut runtime = Self {
            store,
            memory,
            instance,
            exports: HashMap::new(),
            layouts: HashMap::new(),
            callback_index: i32::try_from(index)?,
        };
        let pointer = runtime.call("ghostty_type_json", &[])? as usize;
        let bytes = runtime
            .memory
            .data(&runtime.store)
            .get(pointer..)
            .context("Ghostty layout pointer")?;
        let length = bytes
            .iter()
            .position(|byte| *byte == 0)
            .context("Ghostty layout terminator")?;
        runtime.layouts =
            serde_json::from_slice(&bytes[..length]).context("Ghostty ABI layouts")?;
        Ok(runtime)
    }

    pub fn typed<P: WasmParams, R: WasmResults>(&mut self, name: &str) -> Result<TypedFunc<P, R>> {
        checked(self.instance.get_typed_func(&mut self.store, name))
            .with_context(|| name.to_string())
    }

    pub fn call(&mut self, name: &str, args: &[i32]) -> Result<i32> {
        self.call_values(
            name,
            &args
                .iter()
                .map(|value| Val::I32(*value))
                .collect::<Vec<_>>(),
        )
    }

    pub fn call_values(&mut self, name: &str, args: &[Val]) -> Result<i32> {
        let function = match self.exports.get(name) {
            Some(function) => *function,
            None => {
                let function = self
                    .instance
                    .get_func(&mut self.store, name)
                    .with_context(|| format!("Ghostty export {name}"))?;
                self.exports.insert(name.to_string(), function);
                function
            }
        };
        let mut results = vec![Val::I32(0); function.ty(&self.store).results().len()];
        checked(function.call(&mut self.store, args, &mut results))
            .with_context(|| name.to_string())?;
        Ok(results.first().and_then(Val::i32).unwrap_or(0))
    }

    pub fn success(&mut self, name: &str, args: &[i32]) -> Result<()> {
        let result = self.call(name, args)?;
        ensure!(result == 0, "{name} returned {result}");
        Ok(())
    }

    pub fn alloc(&mut self, size: usize) -> Result<i32> {
        let pointer = self.call("ghostty_wasm_alloc_u8_array", &[i32::try_from(size)?])?;
        ensure!(pointer > 0, "Ghostty allocation of {size} bytes failed");
        self.memory
            .data_mut(&mut self.store)
            .get_mut(pointer as usize..pointer as usize + size)
            .context("Ghostty allocation bounds")?
            .fill(0);
        Ok(pointer)
    }

    pub fn free(&mut self, pointer: i32, size: usize) -> Result<()> {
        if pointer != 0 {
            self.call(
                "ghostty_wasm_free_u8_array",
                &[pointer, i32::try_from(size)?],
            )?;
        }
        Ok(())
    }

    pub fn temporary<T>(
        &mut self,
        size: usize,
        f: impl FnOnce(&mut Self, i32) -> Result<T>,
    ) -> Result<T> {
        let pointer = self.alloc(size)?;
        let result = f(self, pointer);
        let free = self.free(pointer, size);
        match result {
            Ok(value) => {
                free?;
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    pub fn layout(&self, name: &str) -> Result<&Layout> {
        self.layouts
            .get(name)
            .with_context(|| format!("Ghostty layout {name}"))
    }

    pub fn field(&self, name: &str, field: &str) -> Result<&Field> {
        self.layout(name)?
            .fields
            .get(field)
            .with_context(|| format!("Ghostty field {name}.{field}"))
    }

    pub fn set_field(&mut self, pointer: i32, name: &str, field: &str, value: u64) -> Result<()> {
        let field = self.field(name, field)?.clone();
        ensure!(
            matches!(
                field.kind.as_str(),
                "bool" | "u8" | "u16" | "i32" | "u32" | "enum" | "u64"
            ),
            "Unsupported Ghostty field type {}",
            field.kind
        );
        self.memory.write(
            &mut self.store,
            pointer as usize + field.offset,
            &value.to_le_bytes()[..field.size],
        )?;
        Ok(())
    }

    pub fn read_field(&self, pointer: i32, name: &str, field: &str) -> Result<u64> {
        let field = self.field(name, field)?;
        ensure!(field.size <= 8, "Oversized Ghostty integer field");
        let mut bytes = [0; 8];
        self.memory.read(
            &self.store,
            pointer as usize + field.offset,
            &mut bytes[..field.size],
        )?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn bytes(&self, pointer: i32, size: usize) -> Result<&[u8]> {
        self.memory
            .data(&self.store)
            .get(pointer as usize..pointer as usize + size)
            .context("Ghostty memory read")
    }

    pub fn write(&mut self, pointer: i32, bytes: &[u8]) -> Result<()> {
        self.memory
            .write(&mut self.store, pointer as usize, bytes)?;
        Ok(())
    }

    pub fn zero(&mut self, pointer: i32, size: usize) -> Result<()> {
        self.memory
            .data_mut(&mut self.store)
            .get_mut(pointer as usize..pointer as usize + size)
            .context("Ghostty memory clear")?
            .fill(0);
        Ok(())
    }

    pub fn u8(&self, pointer: i32) -> Result<u8> {
        Ok(self.bytes(pointer, 1)?[0])
    }
    pub fn u16(&self, pointer: i32) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(pointer, 2)?.try_into()?))
    }
    pub fn u32(&self, pointer: i32) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(pointer, 4)?.try_into()?))
    }
    pub fn u64(&self, pointer: i32) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(pointer, 8)?.try_into()?))
    }
}
