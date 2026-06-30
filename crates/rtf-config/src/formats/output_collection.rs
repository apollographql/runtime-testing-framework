use crate::{
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
};
use promql_parser::{
    parser::{self, Expr, MatrixSelector, VectorSelector},
    util::{
        parse_duration,
        visitor::{ExprVisitor, walk_expr},
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Defines the data that will be collected when a test execution has completed
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct OutputCollection {
    /// Prometheus queries to execute
    #[serde(default)]
    pub prometheus: Vec<PrometheusQuery>,
}

impl Check for OutputCollection {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut err_path = path.clone();
        err_path.push("output_collection".to_string());

        let mut errs = checks::ErrorBuilder::new();

        for query in self.prometheus.iter() {
            errs.append(query.try_check(&mut err_path, ctx));
        }

        errs.into_result(())
    }
}

impl CheckArrayDuplicates for OutputCollection {
    const BASE_PATH: &str = "output-collection";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, checks::DedupArray<'a>)> {
        vec![("prometheus", DedupArray::Prometheus(&mut self.prometheus))]
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct PrometheusQuery {
    /// The name of the .json file this prometheus query is saved to
    pub name: String,
    /// The query resolution step width. Must be a valid PromQL duration (e.g. `15s`, `1m`, `2h`)
    /// or a float number of seconds (e.g. `15`, `0.5`).
    pub step: String,
    /// The PromQL query to execute
    pub query: String,
}

impl Check for PrometheusQuery {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut err_path = path.clone();
        err_path.push("prometheus".to_string());

        let mut errs = checks::ErrorBuilder::new();

        if parse_duration(&self.step).is_err() {
            errs.push(
                checks::ErrorKind::InvalidDuration,
                format!(
                    "\"{}\" step must be a valid Prometheus duration (e.g. `15s`, `1m`, `2h`) or float seconds (e.g. `15`, `0.5`), got {:?}",
                    self.name, self.step
                ),
                &err_path,
            );
        }

        let promql = match parser::parse(&self.query) {
            Ok(s) => s,
            Err(_) => {
                errs.push(
                    checks::ErrorKind::InvalidPromQl,
                    format!("The \"{}\" query is invalid", self.name),
                    &err_path,
                );

                // Return if the query is not valid PromQL
                return errs.into_result(());
            }
        };

        let mut visitor = NamespaceLabelVisitor::default();
        let _ = walk_expr(&mut visitor, &promql);

        for label in visitor.violations {
            errs.push(
                checks::ErrorKind::ReservedNamespaceLabel,
                format!(
                    "selector '{}' in `{}` contains a reserved 'namespace' label matcher",
                    label, self.name
                ),
                &err_path,
            );
        }

        errs.into_result(())
    }
}

/// Collects the display strings of any VectorSelector or MatrixSelector that contain a
/// `namespace` label matcher.
#[derive(Default)]
struct NamespaceLabelVisitor {
    violations: Vec<String>,
}

impl ExprVisitor for NamespaceLabelVisitor {
    type Error = std::convert::Infallible;

    fn pre_visit(&mut self, expr: &Expr) -> Result<bool, Self::Error> {
        let matchers = match expr {
            Expr::VectorSelector(VectorSelector { matchers, .. }) => matchers,
            Expr::MatrixSelector(MatrixSelector { vs, .. }) => &vs.matchers,
            _ => return Ok(true),
        };

        if matchers.find_matchers("namespace").is_empty() {
            return Ok(true);
        }

        self.violations.push(expr.to_string());

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::Context, formats::tests::assert_check_errors};
    use simple_test_case::test_case;

    fn prom_collection(query: &str) -> PrometheusQuery {
        PrometheusQuery {
            name: "name".to_string(),
            step: "15m".to_string(),
            query: query.to_string(),
        }
    }

    fn prom_collection_with_step(step: &str) -> PrometheusQuery {
        PrometheusQuery {
            name: "name".to_string(),
            step: step.to_string(),
            query: "sum(rate(metric[1m]))".to_string(),
        }
    }

    #[test_case("15s"; "seconds")]
    #[test_case("1m"; "minutes")]
    #[test_case("2h"; "hours")]
    #[test_case("1d"; "days")]
    #[test_case("15"; "float seconds")]
    #[test_case("0.5"; "sub-second float")]
    #[test]
    fn prometheus_collection_step_valid(step: &str) {
        let ctx = Context::new();
        let res = prom_collection_with_step(step).try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test_case(""; "empty string")]
    #[test_case("1.5m"; "float with unit")]
    #[test_case("15 seconds"; "prose duration")]
    #[test]
    fn prometheus_collection_step_invalid(step: &str) {
        let ctx = Context::new();
        assert_check_errors(
            prom_collection_with_step(step),
            &ctx,
            &[checks::ErrorKind::InvalidDuration],
        );
    }

    #[test_case("sum(rate(metric[1m]))"; "one metric no labels")]
    #[test_case("sum(rate(metric{label=\"value\"}[1m]))"; "vector selector with one label")]
    #[test_case("sum(rate(metric{label1=\"value\",label2=\"value\",label3=\"value\"}[1m]))"; "vector selector with multiple labels")]
    #[test]
    fn prometheus_collection_query_valid(query: &str) {
        let prom_collection = prom_collection(query);

        let ctx = Context::new();

        let res = prom_collection.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test_case(
        "not a valid query",
        &[checks::ErrorKind::InvalidPromQl];
        "invalid promql"
    )]
    #[test_case(
        "sum(rate(apollo_router_http_requests_total{namespace=\"abc\"}[1m]))",
        &[checks::ErrorKind::ReservedNamespaceLabel];
        "namespace in vector selector"
    )]
    #[test_case(
        "sum(rate(my_metric{namespace=~\"ns-.*\"}[5m]))",
        &[checks::ErrorKind::ReservedNamespaceLabel];
        "namespace in matrix selector"
    )]
    #[test_case(
        "sum(rate(metric_a{namespace=\"x\"}[1m])) / sum(rate(metric_b{namespace=\"x\"}[1m]))",
        &[checks::ErrorKind::ReservedNamespaceLabel, checks::ErrorKind::ReservedNamespaceLabel];
        "multiple errors"
    )]
    #[test]
    fn prometheus_collection_query_invalid(query: &str, errors: &[checks::ErrorKind]) {
        let prom_collection = prom_collection(query);

        let ctx = Context::new();

        assert_check_errors(prom_collection, &ctx, errors);
    }

    #[test]
    fn output_collection_duplicate_prometheus_names_error() {
        let mut output = OutputCollection {
            prometheus: vec![
                prom_collection("sum(rate(metric[1m]))"),
                prom_collection("sum(rate(metric[1m]))"),
            ],
        };

        let res = output.ensure_no_duplicate_keys();
        assert!(
            res.is_err(),
            "expected error for duplicate prometheus names"
        );
    }

    #[test]
    fn output_collection_dedup_prometheus_keeps_second() {
        let original = PrometheusQuery {
            name: "name".to_string(),
            step: "5m".to_string(),
            query: "sum(rate(metric[1m]))".to_string(),
        };
        let override_query = prom_collection("sum(rate(other_metric[1m]))");

        let mut output = OutputCollection {
            prometheus: vec![original, override_query.clone()],
        };

        let res = output.try_dedup_and_sort();
        assert!(res.is_ok(), "expected OK, got {res:?}");
        assert_eq!(output.prometheus, vec![override_query]);
    }

    #[test]
    fn output_collection_check_success() {
        let output = OutputCollection {
            prometheus: vec![prom_collection("sum(rate(metric[1m]))")],
        };

        let ctx = Context::new();

        let res = output.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn output_collection_check_errors() {
        let output = OutputCollection {
            prometheus: vec![
                prom_collection("sum(rate(metric[1m]))"),
                prom_collection("not a query"),
            ],
        };

        let ctx = Context::new();

        assert_check_errors(output, &ctx, &[checks::ErrorKind::InvalidPromQl]);
    }
}
