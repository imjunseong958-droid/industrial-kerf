//! Models for the tests that plan and make takes: each a closure, in a generator the engine is
//! handed as a host hands it the person's.

use kerf_engine::error::Result;
use kerf_engine::recipe::model::{Catalog, CatalogModel, Generation, Generator, ModelSpec};

/// Models by their names, each with what it takes, whose files `make` writes.
pub struct Models<F> {
    catalog: Catalog,
    make: F,
}

impl<F: Fn(&Generation) -> Result<()>> Generator for Models<F> {
    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn generate(&self, generation: &Generation, _: &mut dyn FnMut(&str)) -> Result<()> {
        (self.make)(generation)
    }
}

/// The models `named`, each with what it takes where that is said, whose files `make` writes.
pub fn models<F: Fn(&Generation) -> Result<()>>(
    named: &[(&str, Option<ModelSpec>)],
    make: F,
) -> Models<F> {
    let models = named
        .iter()
        .map(|(name, spec)| CatalogModel {
            provider: (*name).to_owned(),
            name: (*name).to_owned(),
            spec: spec.clone(),
            by: format!("model {name}"),
        })
        .collect();
    Models {
        catalog: Catalog {
            models,
            ..Catalog::default()
        },
        make,
    }
}

/// A model that answers with its guide: what it was handed is the take. Unix only, as the
/// tests that hand it a guide are.
#[cfg(unix)]
pub fn answers_with_its_guide(generation: &Generation) -> Result<()> {
    let guide = generation
        .request
        .guide
        .as_ref()
        .expect("the step hands the model a guide");
    std::fs::copy(guide, &generation.request.out)?;
    Ok(())
}

/// Models `names`, set up and never run: for a plan.
pub fn not_run(names: &[&str]) -> Models<fn(&Generation) -> Result<()>> {
    let named: Vec<(&str, Option<ModelSpec>)> = names.iter().map(|name| (*name, None)).collect();
    models(&named, |_| panic!("a model set up for a plan is not run"))
}

/// No model at all.
pub fn none() -> Models<fn(&Generation) -> Result<()>> {
    not_run(&[])
}
