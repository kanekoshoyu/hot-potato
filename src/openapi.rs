//! OpenAPI — machine-readable contract for the HTTP surface (RFC-001).
//!
//! The JSON-RPC endpoint is a single POST `/`; its method table is already
//! introspectable at runtime via `rpc.discover`. What OpenAPI adds is the
//! REST-shaped surface: health, `/log`, the agent card, and the documented
//! view of the RPC envelope. Swagger UI mounts at `/docs`.

use utoipa::openapi::path::{
    HttpMethod, Operation, OperationBuilder, Parameter, ParameterBuilder, ParameterIn, Paths,
};
use utoipa::openapi::request_body::RequestBodyBuilder;
use utoipa::openapi::response::ResponseBuilder;
use utoipa::openapi::schema::{ObjectBuilder, SchemaType, Type};
use utoipa::openapi::{
    ComponentsBuilder, ContentBuilder, InfoBuilder, LicenseBuilder, OpenApi, OpenApiBuilder,
};

/// Build the OpenAPI doc. Hand-assembled (rather than derive-macro driven)
/// because the bus surface is one JSON-RPC endpoint + a few typed GETs; the
/// derive macros would document serde_json::Value, which helps nobody.
pub fn openapi_doc() -> OpenApi {
    let mut paths = Paths::new();

    let mut add = |path: &str, method: HttpMethod, op: Operation| {
        paths.add_path_operation(path, vec![method], op);
    };

    add(
        "/health",
        HttpMethod::Get,
        OperationBuilder::new()
            .tag("system")
            .operation_id(Some("getHealth"))
            .summary(Some("Liveness probe"))
            .description(Some("Returns service status."))
            .response("200", ResponseBuilder::new().description("ok"))
            .build(),
    );

    add(
        "/log",
        HttpMethod::Get,
        OperationBuilder::new()
            .tag("observer")
            .operation_id(Some("getLifecycleLog"))
            .summary(Some("Human-readable lifecycle log (F2)"))
            .description(Some(
                "Plain-text chatlog: every letter on the bus, one line each                  (status/sender/receiver/type/id/subject). Read-only.",
            ))
            .parameter(
                ParameterBuilder::new()
                    .name("limit")
                    .parameter_in(ParameterIn::Query)
                    .description(Some("Max letters to show (default 50)"))
                    .schema(Some(
                        ObjectBuilder::new()
                            .schema_type(SchemaType::Type(Type::Integer))
                            .build(),
                    )),
            )
            .response(
                "200",
                ResponseBuilder::new().description("text/plain table"),
            )
            .build(),
    );

    add(
        "/.well-known/agent-card.json",
        HttpMethod::Get,
        OperationBuilder::new()
            .tag("a2a")
            .operation_id(Some("getAgentCard"))
            .summary(Some("A2A v1.0 agent card"))
            .description(Some(
                "Discovery: bus name, endpoint, capabilities, four bus skills.",
            ))
            .response("200", ResponseBuilder::new().description("agent card JSON"))
            .build(),
    );

    add(
        "/",
        HttpMethod::Post,
        OperationBuilder::new()
            .tag("bus")
            .operation_id(Some("jsonRpc"))
            .summary(Some("JSON-RPC 2.0 endpoint — every bus method"))
            .description(Some(
                "Methods: agent/register, message/send, message/poll, message/peek, \
                 message/read, message/ack, agent/status, bus/archive, message/list \
                 (observer), rpc.discover. Replies carry `ref`; broadcasts are PM-only.",
            ))
            .request_body(Some(
                RequestBodyBuilder::new()
                    .content(
                        "application/json",
                        ContentBuilder::new()
                            .example(Some(serde_json::json!({
                                "jsonrpc":"2.0","id":1,
                                "method":"message/send",
                                "params":{"sender":"patricia","receiver":"diana",
                                          "type":"task","subject":"run X","body":"see /path"}
                            })))
                            .build(),
                    )
                    .build(),
            ))
            .response(
                "200",
                ResponseBuilder::new().description("JSON-RPC result or error object"),
            )
            .build(),
    );

    add(
        "/openapi.json",
        HttpMethod::Get,
        OperationBuilder::new()
            .tag("system")
            .operation_id(Some("getOpenapiJson"))
            .summary(Some("This document"))
            .response("200", ResponseBuilder::new().description("OpenAPI JSON"))
            .build(),
    );

    OpenApiBuilder::new()
        .info(
            InfoBuilder::new()
                .title("Hot Potato — EventMessageBus")
                .version(env!("CARGO_PKG_VERSION"))
                .description(Some(
                    "A mailbox with a state machine for AI agents. Agents pass letters \
                     over JSON-RPC 2.0 (POST /); observers read the chatlog (GET /log) \
                     and watch it live (WS /ws). The bus pushes; polling is a fallback.",
                ))
                .license(Some(LicenseBuilder::new().name("MIT").build()))
                .build(),
        )
        .paths(paths)
        .components(Some(ComponentsBuilder::new().build()))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_doc_has_core_paths() {
        let doc = openapi_doc();
        assert!(doc.paths.get_path_item("/health").is_some());
        assert!(doc.paths.get_path_item("/log").is_some());
        assert!(doc.paths.get_path_item("/").is_some());
        assert!(doc
            .paths
            .get_path_item("/.well-known/agent-card.json")
            .is_some());
        assert!(doc
            .paths
            .get_path_operation("/log", HttpMethod::Get)
            .is_some());
    }
}
