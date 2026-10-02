//! The parity guard between the skilld CLI and the skilld.dev public API.
//!
//! The vendored contract lives at `packages/protocol/openapi/skilld-api-v1.json`.
//! `node scripts/sync-api-spec.mjs` refreshes it. This test then fails when:
//!
//! - an operation in the contract has no Rust mapping, or the CLI maps one the
//!   contract lacks;
//! - a mapped method, path, query parameter, access level, or retry rule drifts;
//! - a response example does not parse into the Rust type mapped to it;
//! - a request example names a field the CLI cannot send.

use serde::de::DeserializeOwned;
use serde_json::Value;
use skilld_core::api::{
    Account, AccountChanges, AccountUpdateBody, AddCollectionSkillBody, ApiAccess, ApiAnswerBody,
    ApiList, ApiOperation, CollectionDetail, CollectionSkill, CollectionWatch,
    CreateCollectionBody, CuratorDetail, CuratorSummary, IndexRequest, IndexRequestBody,
    IssuedToken, LikedSkill, OwnerProfile, RepositoryProfile, RepositoryScan, SkillDetail,
    SkillSummary, StarredRepository, StarsImport, StarsImportBody, Token, TokenCreateBody,
    TrackDetail, TrackSummary, TrendingSkill, Watch, operations,
};

const SPEC: &str = include_str!("../../../packages/protocol/openapi/skilld-api-v1.json");

/// Parses one example into a Rust type, or says why it cannot.
type Parser = fn(&Value) -> Result<(), String>;

/// How one operation's 2xx answer parses.
enum Answer {
    Json(Parser),
    Empty,
}

fn parses<T: DeserializeOwned>(value: &Value) -> Result<(), String> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn search(value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    skilld_core::parse_search_response(&bytes)
        .map(|_| ())
        .map_err(|error| error.message)
}

/// The Rust type each operation answers with. An operation the contract adds
/// without a row here fails the test.
fn answer(id: &str) -> Option<Answer> {
    Some(match id {
        "skills.search" => Answer::Json(search),
        "skills.get" => Answer::Json(parses::<SkillDetail>),
        "skills.browse" => Answer::Json(parses::<ApiList<SkillSummary>>),
        "repositories.get" => Answer::Json(parses::<RepositoryProfile>),
        "index_requests.create" | "index_requests.get" => Answer::Json(parses::<IndexRequest>),
        "owners.get" => Answer::Json(parses::<OwnerProfile>),
        "tracks.list" => Answer::Json(parses::<ApiList<TrackSummary>>),
        "tracks.get" => Answer::Json(parses::<TrackDetail>),
        "trending.list" => Answer::Json(parses::<ApiList<TrendingSkill>>),
        "curators.list" => Answer::Json(parses::<ApiList<CuratorSummary>>),
        "curators.get" => Answer::Json(parses::<CuratorDetail>),
        "curators.likes" => Answer::Json(parses::<ApiList<SkillSummary>>),
        "collections.get" | "collections.create" => Answer::Json(parses::<CollectionDetail>),
        "collections.skills.add" => Answer::Json(parses::<CollectionSkill>),
        "collections.skills.remove" => Answer::Empty,
        "collections.watch" => Answer::Json(parses::<CollectionWatch>),
        "account.get" | "account.update" => Answer::Json(parses::<Account>),
        "account.repositories.scan" => Answer::Json(parses::<RepositoryScan>),
        "account.repositories.unpublish" => Answer::Empty,
        "likes.list" => Answer::Json(parses::<ApiList<LikedSkill>>),
        "likes.create" | "likes.delete" => Answer::Empty,
        "watches.list" => Answer::Json(parses::<ApiList<Watch>>),
        "watches.create" | "watches.delete" => Answer::Empty,
        "stars.list" => Answer::Json(parses::<ApiList<StarredRepository>>),
        "stars.import" => Answer::Json(parses::<StarsImport>),
        "changes.list" => Answer::Json(parses::<AccountChanges>),
        "tokens.list" => Answer::Json(parses::<ApiList<Token>>),
        "tokens.create" => Answer::Json(parses::<IssuedToken>),
        "tokens.revoke" => Answer::Empty,
        _ => return None,
    })
}

/// The Rust type each request body serializes from.
fn request(id: &str) -> Option<Parser> {
    Some(match id {
        "index_requests.create" => parses::<IndexRequestBody>,
        "collections.create" => parses::<CreateCollectionBody>,
        "collections.skills.add" => parses::<AddCollectionSkillBody>,
        "account.update" => parses::<AccountUpdateBody>,
        "stars.import" => parses::<StarsImportBody>,
        "tokens.create" => parses::<TokenCreateBody>,
        _ => return None,
    })
}

struct SpecOperation<'a> {
    id: &'a str,
    method: &'a str,
    path: &'a str,
    document: &'a Value,
}

fn spec_operations(spec: &Value) -> Vec<SpecOperation<'_>> {
    let mut found = Vec::new();
    for (path, item) in spec["paths"].as_object().expect("paths") {
        for (method, document) in item.as_object().expect("path item") {
            found.push(SpecOperation {
                id: document["operationId"].as_str().expect("operationId"),
                method,
                path,
                document,
            });
        }
    }
    found
}

fn mapped(id: &str) -> Option<&'static ApiOperation> {
    operations::ALL.iter().find(|operation| operation.id == id)
}

#[test]
fn every_contract_operation_has_a_rust_mapping_and_the_cli_maps_no_other() {
    let spec: Value = serde_json::from_str(SPEC).expect("the vendored contract is JSON");
    let operations = spec_operations(&spec);
    let missing = operations
        .iter()
        .filter(|operation| mapped(operation.id).is_none() || answer(operation.id).is_none())
        .map(|operation| operation.id)
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "map these operations in skilld_core::api and in this test: {missing:?}"
    );
    let stale = operations::ALL
        .iter()
        .filter(|mapped| !operations.iter().any(|operation| operation.id == mapped.id))
        .map(|mapped| mapped.id)
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "the contract no longer has these operations: {stale:?}"
    );
}

#[test]
fn every_mapped_route_matches_the_contract() {
    let spec: Value = serde_json::from_str(SPEC).expect("the vendored contract is JSON");
    for operation in spec_operations(&spec) {
        let Some(rust) = mapped(operation.id) else {
            continue;
        };
        assert_eq!(rust.method.openapi_key(), operation.method, "{}", rust.id);
        assert_eq!(rust.path, operation.path, "{}", rust.id);

        let mut query = operation.document["parameters"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|parameter| parameter["in"] == "query")
            .map(|parameter| parameter["name"].as_str().expect("name"))
            .collect::<Vec<_>>();
        query.sort_unstable();
        let mut rust_query = rust.query.to_vec();
        rust_query.sort_unstable();
        assert_eq!(rust_query, query, "{} query parameters", rust.id);

        let account = operation.document["security"]
            .as_array()
            .is_some_and(|security| !security.is_empty());
        assert_eq!(
            rust.access == ApiAccess::Account,
            account,
            "{} access",
            rust.id
        );

        let semantics = &operation.document["x-skilld-semantics"];
        let retries = semantics["kind"] == "query" || semantics["retry"] == "idempotent";
        assert_eq!(rust.retry, retries, "{} retry rule", rust.id);
    }
}

#[test]
fn every_response_example_parses_into_its_rust_type() {
    let spec: Value = serde_json::from_str(SPEC).expect("the vendored contract is JSON");
    let mut checked = 0;
    for operation in spec_operations(&spec) {
        let (Some(rust), Some(answer)) = (mapped(operation.id), answer(operation.id)) else {
            // The mapping test names this operation.
            continue;
        };
        let success = operation.document["responses"]
            .as_object()
            .expect("responses")
            .iter()
            .filter(|(status, _)| status.starts_with('2'))
            .collect::<Vec<_>>();
        assert_eq!(success.len(), 1, "{} has one success answer", rust.id);
        let (status, response) = success[0];
        let example = response["content"]["application/json"].get("example");
        match (answer, example) {
            (Answer::Json(parse), Some(example)) => {
                assert_eq!(rust.answer, ApiAnswerBody::Json, "{}", rust.id);
                if let Err(error) = parse(example) {
                    panic!("{} {status} example does not parse: {error}", rust.id);
                }
                checked += 1;
            }
            (Answer::Empty, None) => {
                assert_eq!(status, "204", "{}", rust.id);
                assert_eq!(rust.answer, ApiAnswerBody::Empty, "{}", rust.id);
            }
            (Answer::Json(_), None) => panic!("{} {status} has no JSON example", rust.id),
            (Answer::Empty, Some(_)) => panic!("{} {status} answers JSON", rust.id),
        }
    }
    assert!(checked > 0, "the contract has no response examples");
}

#[test]
fn every_request_example_fits_the_body_the_cli_sends() {
    let spec: Value = serde_json::from_str(SPEC).expect("the vendored contract is JSON");
    for operation in spec_operations(&spec) {
        let body = &operation.document["requestBody"];
        if body.is_null() {
            assert!(
                request(operation.id).is_none(),
                "{} takes no body",
                operation.id
            );
            continue;
        }
        let parse = request(operation.id)
            .unwrap_or_else(|| panic!("{} needs a request body type", operation.id));
        if let Some(example) = body["content"]["application/json"].get("example")
            && let Err(error) = parse(example)
        {
            panic!("{} request example does not fit: {error}", operation.id);
        }
    }
}
