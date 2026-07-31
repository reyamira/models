use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::data::{Model, Provider};
use crate::labs::{
    compact_model_id_fingerprint, fingerprint_tokens, full_model_id_fingerprint,
    identity_fingerprint, model_id_fingerprint, outputs_are_disjoint, CanonicalResolutionKind,
    InferenceRejection, ModelIdentity,
};
use crate::provider_category::{provider_category, ProviderCategory};
use crate::tui::app::{App, Message};
use crate::tui::mouse::{hit, row_at};
use crate::tui::widgets::scroll_offset::ScrollOffset;

/// Page size for page up/down navigation
const PAGE_SIZE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Models,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Default,
    #[default]
    ReleaseDate,
    Cost,
    Context,
}

impl SortOrder {
    pub fn next(self) -> Self {
        match self {
            SortOrder::Default => SortOrder::ReleaseDate,
            SortOrder::ReleaseDate => SortOrder::Cost,
            SortOrder::Cost => SortOrder::Context,
            SortOrder::Context => SortOrder::Default,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Filters {
    pub reasoning: bool,
    pub tools: bool,
    pub open_weights: bool,
    pub free: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderListItem {
    All,
    CategoryHeader(ProviderCategory),
    Provider(usize, usize), // (index into providers, match count)
}

#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub model: Model,
    pub provider_id: String,
    /// Projected from the complete catalog snapshot after filters are applied.
    /// Peer provenance must never be recomputed from the visible subset.
    pub identity: Option<ModelIdentityProvenance>,
}

/// Identity provenance for one provider offering inside a grouped row. Peer
/// inference and canonical resolution are both snapshot-global.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelIdentityProvenance {
    AuthoritativeRef,
    AuthoritativeDirectId,
    AuthoritativeScopedId,
    InferredCanonical,
    InferredQualifiedCanonical,
    InferredExactPairCanonical,
    InferredOneSidedCreatorCanonical,
    InferredCrossAliasCanonical,
    InferredFullIdCanonical,
    InferredPeer,
    Unlinked(InferenceRejection),
}

impl ModelIdentityProvenance {
    pub fn is_inferred(self) -> bool {
        matches!(
            self,
            Self::InferredCanonical
                | Self::InferredQualifiedCanonical
                | Self::InferredExactPairCanonical
                | Self::InferredOneSidedCreatorCanonical
                | Self::InferredCrossAliasCanonical
                | Self::InferredFullIdCanonical
                | Self::InferredPeer
        )
    }

    pub fn is_authoritative(self) -> bool {
        matches!(
            self,
            Self::AuthoritativeRef | Self::AuthoritativeDirectId | Self::AuthoritativeScopedId
        )
    }
}

impl From<CanonicalResolutionKind> for ModelIdentityProvenance {
    fn from(kind: CanonicalResolutionKind) -> Self {
        match kind {
            CanonicalResolutionKind::AuthoritativeRef => Self::AuthoritativeRef,
            CanonicalResolutionKind::AuthoritativeDirectId => Self::AuthoritativeDirectId,
            CanonicalResolutionKind::AuthoritativeScopedId => Self::AuthoritativeScopedId,
            CanonicalResolutionKind::InferredCanonical => Self::InferredCanonical,
            CanonicalResolutionKind::InferredQualifiedCanonical => Self::InferredQualifiedCanonical,
            CanonicalResolutionKind::InferredExactPairCanonical => Self::InferredExactPairCanonical,
            CanonicalResolutionKind::InferredOneSidedCreatorCanonical => {
                Self::InferredOneSidedCreatorCanonical
            }
            CanonicalResolutionKind::InferredCrossAliasCanonical => {
                Self::InferredCrossAliasCanonical
            }
            CanonicalResolutionKind::InferredFullIdCanonical => Self::InferredFullIdCanonical,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedGroupIdentity {
    key: String,
    name: String,
    lab: Option<String>,
    provenance: ModelIdentityProvenance,
}

pub struct ModelsApp {
    pub selected_provider: usize,
    pub selected_model: usize,
    pub provider_list_state: ListState,
    pub model_list_state: ListState,
    pub focus: Focus,
    pub sort_order: SortOrder,
    pub sort_ascending: bool,
    pub filters: Filters,
    pub search_query: String,
    pub provider_category_filter: ProviderCategory,
    pub group_by_category: bool,
    pub provider_list_items: Vec<ProviderListItem>,
    filtered_models: Vec<ModelEntry>,
    /// Complete-snapshot identity keyed by `(provider_id, model_id)`. Search,
    /// filters, provider scope, and drill-down only project this stable result;
    /// they never re-run peer conflict checks on a partial view.
    identity_snapshot: std::collections::HashMap<(String, String), ResolvedGroupIdentity>,
    pub detail_scroll: ScrollOffset,
    /// Glossary popup (`i`) explaining the capability/pricing fields.
    pub show_glossary: bool,
    pub glossary_scroll: ScrollOffset,
    /// Panel rects cached at render time for mouse hit-testing (see
    /// `crate::tui::mouse`). The stored areas are the exact rects the list /
    /// detail widgets render into — `provider_list_area`/`model_list_area` are
    /// the bare item regions (no border, no filter row), so `row_at` uses
    /// `top_skip = 0`.
    pub provider_list_area: Option<Rect>,
    pub model_list_area: Option<Rect>,
    pub provider_card_area: Option<Rect>,
    pub model_detail_area: Option<Rect>,
    /// Lab (canonical creator) resolver — set once at startup from
    /// models.dev's canonical registry; defaults to the curated-only table.
    lab_catalog: crate::labs::LabCatalog,
    /// `V` toggle: flat per-offering list instead of the grouped view when
    /// "All" is selected. Persisted via config by the caller.
    pub flat_view: bool,
    /// Push-in drill identity. The key is stable across provider spellings;
    /// the name is the user-facing breadcrumb. `Esc` clears both.
    pub drill_key: Option<String>,
    pub drill_name: Option<String>,
    /// Grouped view rows, rebuilt alongside `filtered_models` whenever the
    /// All scope is active and no drill applies.
    pub groups: Vec<ModelGroup>,
    pub selected_group: usize,
    pub group_list_state: ListState,
    /// Provider picker modal (`p`): search-first provider filter.
    pub show_provider_picker: bool,
    pub picker_query: String,
    pub picker_selected: usize,
    /// Popup inner rect cached at render time for mouse row mapping.
    pub picker_area: std::cell::Cell<Option<Rect>>,
}

/// Which list the center panel is showing. Derived state — see `list_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMode {
    /// One row per canonical model or unlinked provider offering (All scope).
    Grouped,
    /// Offerings of one drilled-into group (breadcrumb view).
    Offerings,
    /// Flat per-offering rows (provider-scoped, or the `V` toggle).
    Flat,
}

/// Aggregate row of the grouped view: one canonical model across its providers,
/// or one provider offering when models.dev supplies no canonical identity.
/// Capability tallies are `(true_count, total)` so the renderer can apply the
/// majority-plus-dim-when-mixed policy.
#[derive(Debug, Clone)]
pub struct ModelGroup {
    /// Stable canonical/offering identity used for drilling and aggregation.
    pub(crate) key: String,
    pub name: String,
    /// Lab slug (via `crate::labs`), `None` when unresolved.
    pub lab: Option<String>,
    /// Distinct providers carrying the model (the "Providers" column — a
    /// provider listing the same name twice counts once).
    pub provider_count: usize,
    pub offering_count: usize,
    pub reasoning: (usize, usize),
    pub tools: (usize, usize),
    pub files: (usize, usize),
    pub open: (usize, usize),
    pub input_range: Option<(f64, f64)>,
    pub output_range: Option<(f64, f64)>,
    pub context_range: Option<(u64, u64)>,
    /// Latest release date across offerings (dates are canonical-inherited,
    /// so usually identical).
    pub max_release: Option<String>,
    /// Index of the first member in `filtered_models` — the representative
    /// offering for detail rendering.
    pub first_entry: usize,
    /// Indices of all members in `filtered_models`, list order.
    pub member_indices: Vec<usize>,
    /// One entry per `member_indices` item, retained so grouped detail can
    /// distinguish models.dev-authored identity from local inference.
    pub member_provenance: Vec<ModelIdentityProvenance>,
}

impl ModelsApp {
    pub fn new(providers: &[(String, Provider)]) -> Self {
        let mut provider_list_state = ListState::default();
        provider_list_state.select(Some(0));
        let mut model_list_state = ListState::default();
        model_list_state.select(Some(0));

        let mut app = Self {
            selected_provider: 0, // Start with "All"
            selected_model: 0,
            provider_list_state,
            model_list_state,
            focus: Focus::Models,
            sort_order: SortOrder::ReleaseDate,
            sort_ascending: false,
            filters: Filters::default(),
            search_query: String::new(),
            provider_category_filter: ProviderCategory::All,
            group_by_category: false,
            provider_list_items: Vec::new(),
            filtered_models: Vec::new(),
            identity_snapshot: std::collections::HashMap::new(),
            detail_scroll: ScrollOffset::default(),
            show_glossary: false,
            glossary_scroll: ScrollOffset::default(),
            provider_list_area: None,
            model_list_area: None,
            provider_card_area: None,
            model_detail_area: None,
            lab_catalog: crate::labs::LabCatalog::default(),
            flat_view: false,
            drill_key: None,
            drill_name: None,
            groups: Vec::new(),
            selected_group: 0,
            group_list_state: {
                let mut s = ListState::default();
                s.select(Some(0));
                s
            },
            show_provider_picker: false,
            picker_query: String::new(),
            picker_selected: 0,
            picker_area: std::cell::Cell::new(None),
        };

        app.update_provider_list(providers);
        app.rebuild_identity_snapshot(providers);
        app.update_filtered_models(providers);
        app
    }

    /// Install the coherent canonical resolver for a new catalog snapshot and
    /// rebuild every offering's identity before any UI filter is projected.
    pub(crate) fn set_lab_catalog(
        &mut self,
        lab_catalog: crate::labs::LabCatalog,
        providers: &[(String, Provider)],
    ) {
        self.lab_catalog = lab_catalog;
        self.rebuild_identity_snapshot(providers);
    }

    pub fn is_all_selected(&self) -> bool {
        matches!(
            self.provider_list_items.get(self.selected_provider),
            Some(ProviderListItem::All)
        )
    }

    pub fn selected_provider_data<'a>(
        &self,
        providers: &'a [(String, Provider)],
    ) -> Option<&'a (String, Provider)> {
        match self.provider_list_items.get(self.selected_provider) {
            Some(ProviderListItem::Provider(idx, _)) => providers.get(*idx),
            _ => None,
        }
    }

    fn has_active_filters(&self) -> bool {
        !self.search_query.is_empty()
            || self.filters.reasoning
            || self.filters.tools
            || self.filters.open_weights
            || self.filters.free
    }

    fn provider_match_count(&self, provider_id: &str, provider: &Provider) -> usize {
        let query_lower = self.search_query.to_lowercase();
        provider
            .models
            .iter()
            .filter(|(model_id, model)| {
                let search_matches = query_lower.is_empty()
                    || model_id.to_lowercase().contains(&query_lower)
                    || model.name.to_lowercase().contains(&query_lower)
                    || provider_id.to_lowercase().contains(&query_lower);
                search_matches && self.passes_filters(model)
            })
            .count()
    }

    pub fn update_provider_list(&mut self, providers: &[(String, Provider)]) {
        self.provider_list_items.clear();
        self.provider_list_items.push(ProviderListItem::All);

        let filtering = self.has_active_filters();

        if self.group_by_category {
            let categories = [
                ProviderCategory::Origin,
                ProviderCategory::Cloud,
                ProviderCategory::Inference,
                ProviderCategory::Gateway,
                ProviderCategory::Tool,
            ];

            for cat in &categories {
                if self.provider_category_filter != ProviderCategory::All
                    && self.provider_category_filter != *cat
                {
                    continue;
                }

                let mut items: Vec<(usize, usize)> = providers
                    .iter()
                    .enumerate()
                    .filter(|(_, (id, _))| provider_category(id) == *cat)
                    .filter_map(|(idx, (id, provider))| {
                        let count = if filtering {
                            let c = self.provider_match_count(id, provider);
                            if c == 0 {
                                return None;
                            }
                            c
                        } else {
                            provider.models.len()
                        };
                        Some((idx, count))
                    })
                    .collect();

                if items.is_empty() {
                    continue;
                }

                items.sort_by(|a, b| providers[a.0].0.cmp(&providers[b.0].0));

                self.provider_list_items
                    .push(ProviderListItem::CategoryHeader(*cat));
                for (idx, count) in items {
                    self.provider_list_items
                        .push(ProviderListItem::Provider(idx, count));
                }
            }
        } else {
            for (idx, (id, provider)) in providers.iter().enumerate() {
                if self.provider_category_filter != ProviderCategory::All
                    && provider_category(id) != self.provider_category_filter
                {
                    continue;
                }
                let count = if filtering {
                    let c = self.provider_match_count(id, provider);
                    if c == 0 {
                        continue;
                    }
                    c
                } else {
                    provider.models.len()
                };
                self.provider_list_items
                    .push(ProviderListItem::Provider(idx, count));
            }
        }
    }

    pub fn find_selectable_index(&self, from: usize, forward: bool) -> usize {
        let len = self.provider_list_items.len();
        if len == 0 {
            return 0;
        }

        let mut idx = from;
        loop {
            if !matches!(
                self.provider_list_items.get(idx),
                Some(ProviderListItem::CategoryHeader(_))
            ) {
                return idx;
            }
            if forward {
                if idx >= len - 1 {
                    return self.find_selectable_index(from.saturating_sub(1), false);
                }
                idx += 1;
            } else {
                if idx == 0 {
                    return 0;
                }
                idx -= 1;
            }
        }
    }

    fn passes_filters(&self, model: &Model) -> bool {
        if self.filters.reasoning && !model.reasoning {
            return false;
        }
        if self.filters.tools && !model.tool_call {
            return false;
        }
        if self.filters.open_weights && !model.open_weights {
            return false;
        }
        if self.filters.free && !model.is_free() {
            return false;
        }
        true
    }

    pub fn update_filtered_models(&mut self, providers: &[(String, Provider)]) {
        let query_lower = self.search_query.to_lowercase();
        let cat_filter = self.provider_category_filter;

        self.filtered_models = if self.is_all_selected() {
            let mut entries: Vec<ModelEntry> = providers
                .iter()
                .filter(|(id, _)| {
                    cat_filter == ProviderCategory::All || provider_category(id) == cat_filter
                })
                .flat_map(|(provider_id, provider)| {
                    provider.models.iter().filter_map(|(model_id, model)| {
                        let search_matches = query_lower.is_empty()
                            || model_id.to_lowercase().contains(&query_lower)
                            || model.name.to_lowercase().contains(&query_lower)
                            || provider_id.to_lowercase().contains(&query_lower);

                        if search_matches && self.passes_filters(model) {
                            Some(ModelEntry {
                                id: model_id.clone(),
                                model: model.clone(),
                                provider_id: provider_id.clone(),
                                identity: None,
                            })
                        } else {
                            None
                        }
                    })
                })
                .collect();

            // Push-in drill: restrict to the stable snapshot identity, never a
            // peer result recomputed from the filtered subset.
            if let Some(drill) = &self.drill_key {
                entries.retain(|entry| {
                    self.snapshot_identity(entry)
                        .is_some_and(|identity| identity.key == *drill)
                });
            }
            self.sort_entries(&mut entries);
            entries
        } else {
            let provider_data = self.selected_provider_data(providers).cloned();
            if let Some((provider_id, provider)) = provider_data {
                let mut entries: Vec<ModelEntry> = provider
                    .models
                    .iter()
                    .filter_map(|(model_id, model)| {
                        let search_matches = query_lower.is_empty()
                            || model_id.to_lowercase().contains(&query_lower)
                            || model.name.to_lowercase().contains(&query_lower);

                        if search_matches && self.passes_filters(model) {
                            Some(ModelEntry {
                                id: model_id.clone(),
                                model: model.clone(),
                                provider_id: provider_id.clone(),
                                identity: None,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                self.sort_entries(&mut entries);
                entries
            } else {
                Vec::new()
            }
        };
        let identities: Vec<_> = self
            .filtered_models
            .iter()
            .map(|entry| {
                self.snapshot_identity(entry)
                    .expect("filtered offering must exist in identity snapshot")
                    .provenance
            })
            .collect();
        for (entry, identity) in self.filtered_models.iter_mut().zip(identities) {
            entry.identity = Some(identity);
        }
        self.rebuild_groups();
    }

    /// Rebuild the grouped rows from the current `filtered_models`. Only
    /// meaningful in the All scope without a drill; cleared otherwise.
    fn rebuild_groups(&mut self) {
        if !self.is_all_selected() || self.drill_key.is_some() {
            // `selected_group` is deliberately preserved here so a drill
            // round-trip (Enter → Esc) restores the user's place.
            self.groups.clear();
            return;
        }
        let mut order: Vec<String> = Vec::new();
        let mut map: std::collections::HashMap<String, ModelGroup> =
            std::collections::HashMap::new();
        let mut providers_seen: std::collections::HashMap<
            String,
            std::collections::HashSet<String>,
        > = std::collections::HashMap::new();
        let identities: Vec<_> = self
            .filtered_models
            .iter()
            .map(|entry| {
                self.snapshot_identity(entry)
                    .expect("grouped offering must exist in identity snapshot")
                    .clone()
            })
            .collect();
        for (idx, (e, identity)) in self.filtered_models.iter().zip(identities).enumerate() {
            let ResolvedGroupIdentity {
                key,
                name: display_name,
                lab: canonical_lab,
                provenance,
            } = identity;
            let g = map.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                ModelGroup {
                    key: key.clone(),
                    name: display_name,
                    lab: canonical_lab.or_else(|| {
                        self.lab_catalog
                            .resolve(&e.model.name, e.model.family.as_deref(), &e.id)
                            .map(String::from)
                    }),
                    provider_count: 0,
                    offering_count: 0,
                    reasoning: (0, 0),
                    tools: (0, 0),
                    files: (0, 0),
                    open: (0, 0),
                    input_range: None,
                    output_range: None,
                    context_range: None,
                    max_release: None,
                    first_entry: idx,
                    member_indices: Vec::new(),
                    member_provenance: Vec::new(),
                }
            });
            g.offering_count += 1;
            g.member_indices.push(idx);
            g.member_provenance.push(provenance);
            providers_seen
                .entry(key)
                .or_default()
                .insert(e.provider_id.clone());
            let m = &e.model;
            let tally = |t: &mut (usize, usize), v: bool| {
                t.1 += 1;
                if v {
                    t.0 += 1;
                }
            };
            tally(&mut g.reasoning, m.reasoning);
            tally(&mut g.tools, m.tool_call);
            tally(&mut g.files, m.attachment);
            tally(&mut g.open, m.open_weights);
            let fold_f = |r: &mut Option<(f64, f64)>, v: Option<f64>| {
                if let Some(v) = v {
                    *r = Some(match *r {
                        Some((lo, hi)) => (lo.min(v), hi.max(v)),
                        None => (v, v),
                    });
                }
            };
            fold_f(&mut g.input_range, m.cost.as_ref().and_then(|c| c.input));
            fold_f(&mut g.output_range, m.cost.as_ref().and_then(|c| c.output));
            if let Some(ctx) = m.limit.as_ref().and_then(|l| l.context) {
                g.context_range = Some(match g.context_range {
                    Some((lo, hi)) => (lo.min(ctx), hi.max(ctx)),
                    None => (ctx, ctx),
                });
            }
            if let Some(d) = &m.release_date {
                if g.max_release.as_deref().is_none_or(|cur| d.as_str() > cur) {
                    g.max_release = Some(d.clone());
                }
            }
        }
        let mut groups: Vec<ModelGroup> = order
            .into_iter()
            .map(|k| {
                let mut g = map.remove(&k).expect("group present");
                g.provider_count = providers_seen.get(&g.key).map_or(0, |s| s.len());
                g
            })
            .collect();
        self.sort_groups(&mut groups);
        self.groups = groups;
        if self.selected_group >= self.groups.len() {
            self.selected_group = self.groups.len().saturating_sub(1);
        }
        self.group_list_state.select(Some(self.selected_group));
    }

    /// Group sort mirrors the flat sort semantics on aggregates: date = max
    /// release, cost = cheapest offering, context = largest window.
    fn sort_groups(&self, groups: &mut [ModelGroup]) {
        let asc = self.sort_ascending;
        match self.sort_order {
            SortOrder::Default => groups.sort_by(|a, b| a.name.cmp(&b.name)),
            SortOrder::ReleaseDate => {
                groups.sort_by(|a, b| match (&b.max_release, &a.max_release) {
                    (Some(bd), Some(ad)) => {
                        if asc {
                            ad.cmp(bd)
                        } else {
                            bd.cmp(ad)
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                })
            }
            SortOrder::Cost => groups.sort_by(|a, b| {
                match (a.input_range.map(|r| r.0), b.input_range.map(|r| r.0)) {
                    (Some(av), Some(bv)) => {
                        let cmp = av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal);
                        if asc {
                            cmp.reverse()
                        } else {
                            cmp
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            }),
            SortOrder::Context => groups.sort_by(|a, b| {
                match (b.context_range.map(|r| r.1), a.context_range.map(|r| r.1)) {
                    (Some(bv), Some(av)) => {
                        if asc {
                            av.cmp(&bv)
                        } else {
                            bv.cmp(&av)
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.name.cmp(&b.name),
                }
            }),
        }
    }

    /// Which list the center panel shows right now.
    pub fn list_mode(&self) -> ListMode {
        if !self.is_all_selected() {
            return ListMode::Flat;
        }
        if self.drill_key.is_some() {
            return ListMode::Offerings;
        }
        if self.flat_view {
            ListMode::Flat
        } else {
            ListMode::Grouped
        }
    }

    pub fn current_group(&self) -> Option<&ModelGroup> {
        self.groups.get(self.selected_group)
    }

    /// Enter on a grouped row: push into its offerings.
    pub fn enter_selection(&mut self, providers: &[(String, Provider)]) {
        if self.list_mode() != ListMode::Grouped {
            return;
        }
        let Some((key, name)) = self
            .current_group()
            .map(|g| (g.key.clone(), g.name.clone()))
        else {
            return;
        };
        self.drill_key = Some(key);
        self.drill_name = Some(name);
        self.selected_model = 0;
        self.model_list_state.select(Some(0));
        self.update_filtered_models(providers);
        self.reset_detail_scroll();
    }

    pub fn open_provider_picker(&mut self) {
        self.show_provider_picker = true;
        self.picker_query.clear();
        self.picker_selected = 0;
    }

    /// Provider-picker rows for the current query: `(provider index in
    /// `providers`, display row)` — "All" is row 0 (`None` index).
    pub fn picker_rows(&self, providers: &[(String, Provider)]) -> Vec<Option<usize>> {
        let q = self.picker_query.to_lowercase();
        let mut rows: Vec<Option<usize>> = vec![None];
        for item in &self.provider_list_items {
            if let ProviderListItem::Provider(idx, _) = item {
                if let Some((id, p)) = providers.get(*idx) {
                    if q.is_empty()
                        || id.to_lowercase().contains(&q)
                        || p.name.to_lowercase().contains(&q)
                    {
                        rows.push(Some(*idx));
                    }
                }
            }
        }
        rows
    }

    /// Apply the picker selection: scope to one provider (or All), close the
    /// modal, and rebuild.
    pub fn apply_picker_selection(&mut self, providers: &[(String, Provider)]) {
        let rows = self.picker_rows(providers);
        let Some(choice) = rows.get(self.picker_selected).copied() else {
            return;
        };
        self.show_provider_picker = false;
        self.drill_key = None;
        self.drill_name = None;
        match choice {
            None => {
                self.selected_provider = 0;
            }
            Some(provider_idx) => {
                // Map the providers-vec index back to its list-item position.
                if let Some(pos) = self.provider_list_items.iter().position(
                    |it| matches!(it, ProviderListItem::Provider(i, _) if *i == provider_idx),
                ) {
                    self.selected_provider = pos;
                }
            }
        }
        self.selected_model = 0;
        self.model_list_state.select(Some(0));
        self.update_filtered_models(providers);
        self.reset_detail_scroll();
    }

    /// Esc, before search-clearing: pop the drill, or provider scope back to
    /// All. Returns true when the key was consumed.
    pub fn escape_back(&mut self, providers: &[(String, Provider)]) -> bool {
        if self.drill_key.is_some() {
            self.drill_key = None;
            self.drill_name = None;
            self.update_filtered_models(providers);
            self.reset_detail_scroll();
            return true;
        }
        if !self.is_all_selected() {
            self.selected_provider = 0;
            self.provider_list_state.select(Some(0));
            self.selected_model = 0;
            self.model_list_state.select(Some(0));
            self.update_filtered_models(providers);
            self.reset_detail_scroll();
            return true;
        }
        false
    }
}

impl ModelsApp {
    fn rebuild_identity_snapshot(&mut self, providers: &[(String, Provider)]) {
        let entries: Vec<ModelEntry> = providers
            .iter()
            .flat_map(|(provider_id, provider)| {
                provider.models.iter().map(|(model_id, model)| ModelEntry {
                    id: model_id.clone(),
                    model: model.clone(),
                    provider_id: provider_id.clone(),
                    identity: None,
                })
            })
            .collect();
        let identities = self.resolve_group_identities(&entries);
        self.identity_snapshot = entries
            .into_iter()
            .zip(identities)
            .map(|(entry, identity)| ((entry.provider_id, entry.id), identity))
            .collect();
    }

    fn snapshot_identity(&self, entry: &ModelEntry) -> Option<&ResolvedGroupIdentity> {
        self.identity_snapshot
            .get(&(entry.provider_id.clone(), entry.id.clone()))
    }

    /// Resolve every entry as a set so conservative peer grouping can validate
    /// whole buckets (never transitive pairwise unions). Canonical resolution
    /// remains independently decided by `LabCatalog` first.
    fn resolve_group_identities(&self, entries: &[ModelEntry]) -> Vec<ResolvedGroupIdentity> {
        let mut identities: Vec<ResolvedGroupIdentity> = entries
            .iter()
            .map(|entry| {
                match self.lab_catalog.resolve_model_identity(
                    &entry.provider_id,
                    &entry.id,
                    &entry.model,
                ) {
                    ModelIdentity::Canonical(resolution) => ResolvedGroupIdentity {
                        key: format!("model:{}", resolution.id),
                        name: resolution.name.to_string(),
                        lab: Some(resolution.lab.to_string()),
                        provenance: resolution.kind.into(),
                    },
                    ModelIdentity::Unlinked(reason) => ResolvedGroupIdentity {
                        key: format!("offering:{}/{}", entry.provider_id, entry.id),
                        name: if entry.model.name.is_empty() {
                            entry.id.clone()
                        } else {
                            entry.model.name.clone()
                        },
                        lab: None,
                        provenance: ModelIdentityProvenance::Unlinked(reason),
                    },
                }
            })
            .collect();

        // Stronger peer lane: the provider-independent leaf id agrees. This
        // preserves the existing behavior for namespaced and bare ids.
        self.apply_peer_groups(entries, &mut identities, "leaf", model_id_fingerprint);

        // Weaker fallback, only for offerings still unlinked after the leaf
        // lane: compare the complete id so provider-specific namespace
        // serialization (`creator/model`, `creator-model`, `creator.model`)
        // can agree. Keeping the lanes sequential prevents this fallback from
        // splitting or transitively expanding a stronger peer group.
        self.apply_peer_groups(entries, &mut identities, "full", full_model_id_fingerprint);

        // Weakest fallback: remove separator boundaries from the complete id
        // only after both token-preserving lanes fail. Name agreement and all
        // bucket-level creator/modality blockers still apply.
        self.apply_peer_groups(
            entries,
            &mut identities,
            "compact",
            compact_model_id_fingerprint,
        );

        // Final relaxation: attach a still-unlinked offering to an existing
        // peer bucket that already agrees on its exact leaf id, when every
        // name-token difference is provably non-identity: creator attribution
        // (the bucket lab's slug/display tokens), namespace spelling from the
        // offering's own id path, or a token the shared leaf id itself
        // carries (an id-echo like "instruct"/"it" cannot distinguish
        // offerings whose complete leaf ids are identical). Semantic tokens
        // from nowhere — dates, "preview", "thinking", sizes — keep the
        // offering out.
        self.apply_peer_name_relaxation(entries, &mut identities);

        identities
    }

    fn apply_peer_name_relaxation(
        &self,
        entries: &[ModelEntry],
        identities: &mut [ResolvedGroupIdentity],
    ) {
        struct Bucket {
            key: String,
            display_name: String,
            lab: String,
            name_tokens: std::collections::HashSet<String>,
            member_indices: Vec<usize>,
        }

        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (idx, identity) in identities.iter().enumerate() {
            if matches!(identity.provenance, ModelIdentityProvenance::InferredPeer) {
                groups.entry(identity.key.clone()).or_default().push(idx);
            }
        }

        // Only buckets whose members all spell one exact leaf id are anchors,
        // and only when the bucket carries an unambiguous creator — without a
        // lab there is no creator vocabulary, and provider-branded pseudo
        // models ("Pioneer Auto") must not attract each other.
        let mut buckets_by_leaf: std::collections::HashMap<String, Vec<Bucket>> =
            std::collections::HashMap::new();
        for (key, member_indices) in groups {
            let mut leafs = member_indices
                .iter()
                .map(|&idx| model_id_fingerprint(&entries[idx].id));
            let leaf = leafs.next().expect("peer bucket has members");
            if leaf.is_empty() || !leafs.all(|other| other == leaf) {
                continue;
            }
            let Some(lab) = identities[member_indices[0]].lab.clone() else {
                continue;
            };
            let name_tokens = fingerprint_tokens(&identity_fingerprint(
                &entries[member_indices[0]].model.name,
            ));
            if name_tokens.is_empty() {
                continue;
            }
            buckets_by_leaf.entry(leaf).or_default().push(Bucket {
                key,
                display_name: identities[member_indices[0]].name.clone(),
                lab,
                name_tokens,
                member_indices,
            });
        }

        let entry_outputs = |idx: usize| -> &[String] {
            entries[idx]
                .model
                .modalities
                .as_ref()
                .map(|modalities| modalities.output.as_slice())
                .unwrap_or_default()
        };

        let mut admitted: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        let mut assignments: Vec<(usize, String, usize)> = Vec::new();
        for (idx, identity) in identities.iter().enumerate() {
            if !matches!(identity.provenance, ModelIdentityProvenance::Unlinked(_)) {
                continue;
            }
            let entry = &entries[idx];
            let leaf = model_id_fingerprint(&entry.id);
            let Some(buckets) = buckets_by_leaf.get(&leaf) else {
                continue;
            };
            let name_tokens = fingerprint_tokens(&identity_fingerprint(&entry.model.name));
            if name_tokens.is_empty() {
                continue;
            }
            let leaf_tokens = fingerprint_tokens(&leaf);
            let namespace_tokens: std::collections::HashSet<String> = entry
                .id
                .rsplit_once('/')
                .map(|(namespace, _)| fingerprint_tokens(&identity_fingerprint(namespace)))
                .unwrap_or_default();

            let matching: Vec<usize> = buckets
                .iter()
                .enumerate()
                .filter(|(_, bucket)| {
                    let vocabulary = self.lab_catalog.creator_alias_tokens(&bucket.lab);
                    let neutral =
                        |token: &String| leaf_tokens.contains(token) || vocabulary.contains(token);
                    bucket.name_tokens.difference(&name_tokens).all(&neutral)
                        && name_tokens
                            .difference(&bucket.name_tokens)
                            .all(|token| neutral(token) || namespace_tokens.contains(token))
                })
                .map(|(position, _)| position)
                .collect();
            // More than one neutral-compatible bucket is ambiguity — refuse.
            let [position] = matching[..] else {
                continue;
            };
            let bucket = &buckets[position];

            if self
                .lab_catalog
                .independent_lab(&entry.provider_id, &entry.id)
                .is_some_and(|lab| lab != bucket.lab)
            {
                continue;
            }
            let peers_conflict = bucket
                .member_indices
                .iter()
                .chain(admitted.get(&bucket.key).into_iter().flatten())
                .any(|&member| outputs_are_disjoint(entry_outputs(idx), entry_outputs(member)));
            if peers_conflict {
                continue;
            }

            admitted.entry(bucket.key.clone()).or_default().push(idx);
            assignments.push((idx, leaf, position));
        }

        for (idx, leaf, position) in assignments {
            let bucket = &buckets_by_leaf[&leaf][position];
            identities[idx].key.clone_from(&bucket.key);
            identities[idx].name.clone_from(&bucket.display_name);
            identities[idx].lab = Some(bucket.lab.clone());
            identities[idx].provenance = ModelIdentityProvenance::InferredPeer;
        }
    }

    fn apply_peer_groups(
        &self,
        entries: &[ModelEntry],
        identities: &mut [ResolvedGroupIdentity],
        lane: &str,
        id_fingerprint: fn(&str) -> String,
    ) {
        let mut peer_candidates: std::collections::HashMap<(String, String), Vec<usize>> =
            std::collections::HashMap::new();
        for (idx, (entry, identity)) in entries.iter().zip(identities.iter()).enumerate() {
            if !matches!(identity.provenance, ModelIdentityProvenance::Unlinked(_)) {
                continue;
            }
            let name = identity_fingerprint(&entry.model.name);
            let id = id_fingerprint(&entry.id);
            if !name.is_empty() && !id.is_empty() {
                peer_candidates.entry((name, id)).or_default().push(idx);
            }
        }

        for ((name_fingerprint, id_fingerprint), indices) in peer_candidates {
            let providers: std::collections::HashSet<&str> = indices
                .iter()
                .map(|&idx| entries[idx].provider_id.as_str())
                .collect();
            if providers.len() < 2 {
                continue;
            }

            let labs: std::collections::HashSet<&str> = indices
                .iter()
                .filter_map(|&idx| {
                    let entry = &entries[idx];
                    self.lab_catalog
                        .independent_lab(&entry.provider_id, &entry.id)
                })
                .collect();
            if labs.len() > 1 {
                continue;
            }

            let outputs_conflict = indices.iter().enumerate().any(|(left_pos, &left_idx)| {
                let left = entries[left_idx]
                    .model
                    .modalities
                    .as_ref()
                    .map(|modalities| modalities.output.as_slice())
                    .unwrap_or_default();
                indices.iter().skip(left_pos + 1).any(|&right_idx| {
                    let right = entries[right_idx]
                        .model
                        .modalities
                        .as_ref()
                        .map(|modalities| modalities.output.as_slice())
                        .unwrap_or_default();
                    outputs_are_disjoint(left, right)
                })
            });
            if outputs_conflict {
                continue;
            }

            let key = format!("peer:{lane}:{name_fingerprint}|{id_fingerprint}");
            let lab = labs.into_iter().next().map(String::from);
            for idx in indices {
                identities[idx].key.clone_from(&key);
                identities[idx].lab.clone_from(&lab);
                identities[idx].provenance = ModelIdentityProvenance::InferredPeer;
            }
        }
    }
}

impl ModelsApp {
    fn sort_entries(&self, entries: &mut [ModelEntry]) {
        match self.sort_order {
            SortOrder::Default => {
                entries.sort_by(|a, b| a.provider_id.cmp(&b.provider_id).then(a.id.cmp(&b.id)));
            }
            SortOrder::ReleaseDate => {
                entries.sort_by(
                    |a, b| match (&b.model.release_date, &a.model.release_date) {
                        (Some(b_date), Some(a_date)) => {
                            if self.sort_ascending {
                                a_date.cmp(b_date)
                            } else {
                                b_date.cmp(a_date)
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.id.cmp(&b.id),
                    },
                );
            }
            SortOrder::Cost => {
                entries.sort_by(|a, b| {
                    let a_cost = a.model.cost.as_ref().and_then(|c| c.input);
                    let b_cost = b.model.cost.as_ref().and_then(|c| c.input);
                    match (a_cost, b_cost) {
                        (Some(a_val), Some(b_val)) => {
                            let cmp = a_val
                                .partial_cmp(&b_val)
                                .unwrap_or(std::cmp::Ordering::Equal);
                            if self.sort_ascending {
                                cmp.reverse()
                            } else {
                                cmp
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.id.cmp(&b.id),
                    }
                });
            }
            SortOrder::Context => {
                entries.sort_by(|a, b| {
                    let a_ctx = a.model.limit.as_ref().and_then(|l| l.context);
                    let b_ctx = b.model.limit.as_ref().and_then(|l| l.context);
                    match (b_ctx, a_ctx) {
                        (Some(b_val), Some(a_val)) => {
                            if self.sort_ascending {
                                a_val.cmp(&b_val)
                            } else {
                                b_val.cmp(&a_val)
                            }
                        }
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => a.id.cmp(&b.id),
                    }
                });
            }
        }
    }

    pub fn select_provider_at_index(&mut self, index: usize, providers: &[(String, Provider)]) {
        self.selected_provider = index;
        self.selected_model = 0;
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model));
        // +1 for header
        self.reset_detail_scroll();
    }

    /// The entry the detail panel keys off. In grouped mode this is the
    /// selected group's representative (first) offering.
    pub fn current_model(&self) -> Option<&ModelEntry> {
        if self.list_mode() == ListMode::Grouped {
            self.current_group()
                .and_then(|g| self.filtered_models.get(g.first_entry))
        } else {
            self.filtered_models.get(self.selected_model)
        }
    }

    pub fn filtered_models(&self) -> &[ModelEntry] {
        &self.filtered_models
    }

    #[cfg(test)]
    pub(crate) fn reconciliation_evidence<'a>(
        &'a self,
        entry: &ModelEntry,
    ) -> Option<crate::labs::ReconciliationEvidence<'a>> {
        self.lab_catalog
            .reconciliation_evidence(&entry.provider_id, &entry.id, &entry.model)
    }

    pub fn get_copy_full(&self) -> Option<String> {
        self.current_model()
            .map(|entry| format!("{}/{}", entry.provider_id, entry.id))
    }

    pub fn get_copy_model_id(&self) -> Option<String> {
        self.current_model().map(|entry| entry.id.clone())
    }

    pub fn get_provider_doc(&self, providers: &[(String, Provider)]) -> Option<String> {
        self.current_model().and_then(|entry| {
            providers
                .iter()
                .find(|(id, _)| id == &entry.provider_id)
                .and_then(|(_, provider)| provider.doc.clone())
        })
    }

    pub fn get_provider_api(&self, providers: &[(String, Provider)]) -> Option<String> {
        self.current_model().and_then(|entry| {
            providers
                .iter()
                .find(|(id, _)| id == &entry.provider_id)
                .and_then(|(_, provider)| provider.api.clone())
        })
    }

    // --- Navigation handlers called from App::update ---

    /// Row count of whichever list the center panel is showing (public for
    /// the mouse handler's row mapping).
    pub fn mouse_row_count(&self) -> usize {
        self.nav_len()
    }

    /// Row count of whichever list the center panel is showing.
    fn nav_len(&self) -> usize {
        if self.list_mode() == ListMode::Grouped {
            self.groups.len()
        } else {
            self.filtered_models.len()
        }
    }

    fn nav_selected(&self) -> usize {
        if self.list_mode() == ListMode::Grouped {
            self.selected_group
        } else {
            self.selected_model
        }
    }

    fn nav_select(&mut self, idx: usize) {
        if self.list_mode() == ListMode::Grouped {
            self.selected_group = idx;
            self.group_list_state.select(Some(idx));
        } else {
            self.selected_model = idx;
            self.model_list_state.select(Some(idx));
        }
        self.reset_detail_scroll();
    }

    pub fn next_model(&mut self) {
        if self.nav_selected() < self.nav_len().saturating_sub(1) {
            self.nav_select(self.nav_selected() + 1);
        }
    }

    pub fn prev_model(&mut self) {
        if self.nav_selected() > 0 {
            self.nav_select(self.nav_selected() - 1);
        }
    }

    /// Select a row by its index into the visible list (used by mouse clicks).
    pub fn select_model_at_index(&mut self, index: usize) {
        if index < self.nav_len() && index != self.nav_selected() {
            self.nav_select(index);
        }
    }

    pub fn select_first_model(&mut self) {
        if self.nav_selected() > 0 {
            self.nav_select(0);
        }
    }

    pub fn select_last_model(&mut self) {
        let last = self.nav_len().saturating_sub(1);
        if self.nav_selected() < last {
            self.nav_select(last);
        }
    }

    pub fn page_down_model(&mut self) {
        let last_index = self.nav_len().saturating_sub(1);
        let next = (self.nav_selected() + PAGE_SIZE).min(last_index);
        if next != self.nav_selected() {
            self.nav_select(next);
        }
    }

    pub fn page_up_model(&mut self) {
        let next = self.nav_selected().saturating_sub(PAGE_SIZE);
        if next != self.nav_selected() {
            self.nav_select(next);
        }
    }

    pub fn focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::Models => Focus::Details,
            Focus::Details => Focus::Models,
        };
    }

    pub fn focus_left(&mut self) {
        // Two panels: left/right both toggle.
        self.focus_right();
    }

    pub fn reset_detail_scroll(&self) {
        self.detail_scroll.jump_top();
    }

    pub fn toggle_glossary(&mut self) {
        self.show_glossary = !self.show_glossary;
        if self.show_glossary {
            self.glossary_scroll.jump_top();
        }
    }

    pub fn scroll_glossary_down(&self) {
        self.glossary_scroll.increment(1);
    }

    pub fn scroll_glossary_up(&self) {
        self.glossary_scroll.decrement(1);
    }

    pub fn cycle_sort(&mut self, providers: &[(String, Provider)]) {
        self.sort_order = self.sort_order.next();
        self.sort_ascending = false;
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model));
        self.reset_detail_scroll();
    }

    pub fn toggle_sort_dir(&mut self, providers: &[(String, Provider)]) {
        if self.sort_order != SortOrder::Default {
            self.sort_ascending = !self.sort_ascending;
            self.selected_model = 0;
            self.update_filtered_models(providers);
            self.model_list_state.select(Some(self.selected_model));
            self.reset_detail_scroll();
        }
    }

    pub fn toggle_reasoning(&mut self, providers: &[(String, Provider)]) {
        self.filters.reasoning = !self.filters.reasoning;
        self.rebuild_after_filter_change(providers);
    }

    pub fn toggle_tools(&mut self, providers: &[(String, Provider)]) {
        self.filters.tools = !self.filters.tools;
        self.rebuild_after_filter_change(providers);
    }

    pub fn toggle_open_weights(&mut self, providers: &[(String, Provider)]) {
        self.filters.open_weights = !self.filters.open_weights;
        self.rebuild_after_filter_change(providers);
    }

    pub fn toggle_free(&mut self, providers: &[(String, Provider)]) {
        self.filters.free = !self.filters.free;
        self.rebuild_after_filter_change(providers);
    }

    pub fn cycle_provider_category(&mut self, providers: &[(String, Provider)]) {
        self.provider_category_filter = self.provider_category_filter.next();
        self.update_provider_list(providers);
        self.selected_provider = self.find_selectable_index(0, true);
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model));
        self.reset_detail_scroll();
    }

    pub fn toggle_grouping(&mut self, providers: &[(String, Provider)]) {
        self.group_by_category = !self.group_by_category;
        self.update_provider_list(providers);
        self.selected_provider = self.find_selectable_index(0, true);
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model));
        self.reset_detail_scroll();
    }

    pub fn search_input(&mut self, c: char, providers: &[(String, Provider)]) {
        self.search_query.push(c);
        self.rebuild_after_filter_change(providers);
    }

    pub fn search_backspace(&mut self, providers: &[(String, Provider)]) {
        self.search_query.pop();
        self.rebuild_after_filter_change(providers);
    }

    pub fn clear_search(&mut self, providers: &[(String, Provider)]) {
        self.search_query.clear();
        self.rebuild_after_filter_change(providers);
    }

    /// Rebuild provider list and model list after any search/filter change.
    /// Preserves the selected provider if it's still visible, otherwise falls back to "All".
    fn rebuild_after_filter_change(&mut self, providers: &[(String, Provider)]) {
        // Remember which provider was selected (by index into providers slice)
        let prev_provider_idx = match self.provider_list_items.get(self.selected_provider) {
            Some(ProviderListItem::Provider(idx, _)) => Some(*idx),
            _ => None, // All or CategoryHeader
        };

        self.update_provider_list(providers);

        // Try to find the previously selected provider in the new list
        let new_pos = prev_provider_idx.and_then(|prev_idx| {
            self.provider_list_items.iter().position(
                |item| matches!(item, ProviderListItem::Provider(idx, _) if *idx == prev_idx),
            )
        });

        self.selected_provider = new_pos.unwrap_or(0);
        self.provider_list_state
            .select(Some(self.selected_provider));
        self.selected_model = 0;
        self.update_filtered_models(providers);
        self.model_list_state.select(Some(self.selected_model));
        self.reset_detail_scroll();
    }
}

/// Handle a mouse event while the Models tab is active.
///
/// All state changes (focus, selection, scroll) are applied directly to `app`,
/// so the function returns `None`; the main loop redraws after every event. The
/// `Option<Message>` return keeps the per-tab handler signature uniform with the
/// other tabs' dispatchers.
pub fn handle_models_mouse(app: &mut App, ev: MouseEvent) -> Option<Message> {
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if hit(app.models_app.model_list_area, &ev) {
                app.models_app.focus = Focus::Models;
                if let Some(area) = app.models_app.model_list_area {
                    // The cached rect is the bare row region (the column
                    // header is a fixed line ABOVE the list, so rows map 1:1).
                    // The state/row-count pair is mode-dependent (grouped list
                    // vs flat offerings) — `nav_*` dispatches identically.
                    let offset = if app.models_app.list_mode() == super::app::ListMode::Grouped {
                        app.models_app.group_list_state.offset()
                    } else {
                        app.models_app.model_list_state.offset()
                    };
                    if let Some(idx) =
                        row_at(area, offset, 0, app.models_app.mouse_row_count(), ev.row)
                    {
                        app.models_app.select_model_at_index(idx);
                    }
                }
            } else if hit(app.models_app.model_detail_area, &ev)
                || hit(app.models_app.provider_card_area, &ev)
            {
                app.models_app.focus = Focus::Details;
            }
        }
        // Wheel: focus the panel under the cursor, then scroll it (reusing the
        // same per-panel nav the arrow keys drive).
        MouseEventKind::ScrollDown => {
            if hit(app.models_app.model_list_area, &ev) {
                app.models_app.focus = Focus::Models;
                app.models_app.next_model();
            } else if hit(app.models_app.model_detail_area, &ev)
                || hit(app.models_app.provider_card_area, &ev)
            {
                app.models_app.focus = Focus::Details;
                app.models_app.detail_scroll.increment(1);
            }
        }
        MouseEventKind::ScrollUp => {
            if hit(app.models_app.model_list_area, &ev) {
                app.models_app.focus = Focus::Models;
                app.models_app.prev_model();
            } else if hit(app.models_app.model_detail_area, &ev)
                || hit(app.models_app.provider_card_area, &ev)
            {
                app.models_app.focus = Focus::Details;
                app.models_app.detail_scroll.decrement(1);
            }
        }
        _ => {}
    }
    None
}
