//! Run the trusted, built-in v1 authority. User releases supply only JSON
//! inputs. No activity source, native modules, filesystem or network API is
//! ever registered in the embedded interpreter.
use rquickjs::{Context, Function, Module, Runtime};
use serde_json::Value;

use super::Manifest;
use crate::{http::ApiError, values::Id};

pub(super) struct Authority {
    context: Context,
}
const ROOM: &str = include_str!("../../../../../sdk/dev/room.js");

impl Authority {
    pub fn new(manifest: &Manifest) -> Result<Self, ApiError> {
        let runtime = Runtime::new().map_err(ApiError::internal)?;
        runtime.set_memory_limit(32 << 20);
        runtime.set_max_stack_size(512 << 10);
        let context = Context::full(&runtime).map_err(ApiError::internal)?;
        context.with(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();
            globals.set("uuid", Function::new(ctx.clone(), || Id::random().to_string())?)?;
            globals.set("utf8ByteLength", Function::new(ctx.clone(), |s: String| s.len() as u32)?)?;
            globals.set("options", serde_json::to_string(manifest).expect("serializable manifest"))?;
            let (module, ready) = Module::declare(ctx.clone(), "room", ROOM)?.eval()?;
            ready.finish::<()>()?;
            globals.set("createRoom", module.get::<_, Function>("createRoom")?)?;
            ctx.eval::<(), _>(r#"
                globalThis.crypto = { randomUUID: uuid };
                const room = createRoom(JSON.parse(options));
                globalThis.run = (person, input) => {
                    try { return JSON.stringify({ value: room(JSON.parse(person), JSON.parse(input)) }); }
                    catch(e) { return JSON.stringify({ status: e.status || 500, error: String(e.message) }); }
                };
                globalThis.revoke = id => room.revoke(id);
                globalThis.expired = () => room.expired();
            "#)
        }).map_err(ApiError::internal)?;
        Ok(Self { context })
    }
    pub fn run(&self, person: &Value, input: &Value) -> Result<Value, ApiError> {
        let output = self
            .context
            .with(|ctx| -> rquickjs::Result<String> {
                ctx.globals()
                    .get::<_, Function>("run")?
                    .call((person.to_string(), input.to_string()))
            })
            .map_err(ApiError::internal)?;
        serde_json::from_str(&output).map_err(ApiError::internal)
    }
    pub fn revoke(&self, id: Id) {
        let _ = self.context.with(|ctx| -> rquickjs::Result<()> {
            ctx.globals()
                .get::<_, Function>("revoke")?
                .call((id.to_string(),))
        });
    }
    pub fn expired(&self) -> bool {
        self.context
            .with(|ctx| -> rquickjs::Result<bool> {
                ctx.globals().get::<_, Function>("expired")?.call(())
            })
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trusted_room_runs_on_native_api_with_private_hosted_results() {
        let manifest = serde_json::from_value(json!({
            "protocol":1,"id":"cards","name":"Cards","description":"Table","version":"1.3.0",
            "authority":"host","seats":2
        }))
        .unwrap();
        let room = Authority::new(&manifest).unwrap();
        let call = |user: &str, frame: Id, mut input: Value| {
            input["frame"] = json!(frame);
            room.run(&json!({"id":user,"name":user}), &input).unwrap()
        };
        let alice = Id::random();
        let bob = Id::random();
        let carol = Id::random();
        let open = json!({"op":"open","initialState":{"points":0}});
        let opened = call("alice", alice, open.clone());
        assert_eq!(opened["value"]["host"], "alice", "{opened}");
        let epoch = &opened["value"]["epoch"];
        call("bob", bob, open.clone());
        let spectator = call("carol", carol, open);
        assert_eq!(spectator["value"]["seats"], json!(["alice", "bob"]));
        let command = json!({"op":"submit","kind":"play","data":"a","id":"once"});
        assert_eq!(call("carol", carol, command.clone())["status"], 403);
        assert_eq!(call("bob", bob, command.clone())["value"]["pending"], true);
        let decided = call(
            "alice",
            alice,
            json!({"op":"decide","id":"once","from":"bob","epoch":epoch,
            "expected":0,"patch":{"points":1},"assignments":{"bob":"private"},"result":"accepted"}),
        );
        assert_eq!(decided["value"]["revision"], 1, "{decided}");
        call(
            "alice",
            alice,
            json!({"op":"transfer","to":"bob","epoch":epoch}),
        );
        assert_eq!(
            call("bob", bob, command)["value"],
            json!({"revision":1,"result":"accepted"})
        );
        assert!(
            !call("carol", carol, json!({"op":"read","after":0}))
                .to_string()
                .contains("private")
        );
        assert_eq!(
            call("bob", bob, json!({"op":"read","after":0}))["value"]["assignment"],
            "private"
        );
    }
}
