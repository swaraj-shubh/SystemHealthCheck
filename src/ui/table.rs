//! Sortable, searchable table on GtkColumnView. Rows are `Vec<String>`;
//! numeric columns sort by value (understands %, units and byte sizes).

use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy)]
pub struct Col {
    pub title: &'static str,
    pub numeric: bool,
    pub expand: bool,
}

pub const fn col(title: &'static str) -> Col {
    Col { title, numeric: false, expand: false }
}
pub const fn num(title: &'static str) -> Col {
    Col { title, numeric: true, expand: false }
}
pub const fn wide(title: &'static str) -> Col {
    Col { title, numeric: false, expand: true }
}

/// Sort key for numeric cells: "1.5 GiB" -> bytes, "12.5%" -> 12.5, "—" -> -inf.
pub fn sort_key(s: &str) -> f64 {
    let s = s.trim();
    let Some(n) = systemhealthcheck::parsers::leading_num(s) else { return f64::NEG_INFINITY };
    let unit = s.trim_start_matches(|c: char| c.is_ascii_digit() || ".,-+ ".contains(c));
    let mult = match unit.split_whitespace().next().unwrap_or("") {
        "KiB" | "K" | "kB" => 1024.0,
        "MiB" | "M" | "MB" => 1024f64.powi(2),
        "GiB" | "G" | "GB" => 1024f64.powi(3),
        "TiB" | "T" | "TB" => 1024f64.powi(4),
        "ms" => 1e-3,
        "min" => 60.0,
        _ => 1.0,
    };
    n * mult
}

/// One row: its cells plus the labels currently showing it, so a refresh
/// updates visible text in place instead of recreating row widgets.
#[derive(Default)]
struct RowData {
    cells: Vec<String>,
    labels: Vec<(usize, glib::WeakRef<gtk::Label>)>,
}

fn cells(o: &glib::Object) -> Vec<String> {
    o.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<RowData>().cells.clone()).unwrap_or_default()
}

/// Row objects are kept stable (keyed by `key_col`) across refreshes and the
/// model is only spliced when the visible order changes. Replacing every item
/// each second made GTK rebuild all row widgets (~30% of a core for the
/// process table); this keeps a 1 s refresh cheap. Sorting happens in Rust.
pub struct DataTable {
    pub widget: gtk::Box,
    pub view: gtk::ColumnView,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    key_col: usize,
    count: gtk::Label,
    numeric: Vec<bool>,
    rows: RefCell<Vec<Vec<String>>>,
    objects: RefCell<std::collections::HashMap<String, glib::BoxedAnyObject>>,
}

impl DataTable {
    /// `key_col` identifies a row across refreshes (selection is kept).
    pub fn new(cols: &[Col], searchable: bool, key_col: usize, height: i32) -> Rc<DataTable> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let query = Rc::new(RefCell::new(String::new()));
        let q2 = query.clone();
        let filter = gtk::CustomFilter::new(move |obj| {
            let q = q2.borrow();
            if q.is_empty() {
                return true;
            }
            obj.downcast_ref::<glib::BoxedAnyObject>()
                .map(|b| b.borrow::<RowData>().cells.iter().any(|c| c.to_lowercase().contains(q.as_str())))
                .unwrap_or(true)
        });
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let view = gtk::ColumnView::new(None::<gtk::SingleSelection>);
        view.set_show_row_separators(true);
        view.set_reorderable(false);
        view.add_css_class("data-table");
        for (i, c) in cols.iter().enumerate() {
            let factory = gtk::SignalListItemFactory::new();
            let numeric = c.numeric;
            factory.connect_setup(move |_, item| {
                let label = gtk::Label::new(None);
                label.set_xalign(if numeric { 1.0 } else { 0.0 });
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                if let Some(li) = item.downcast_ref::<gtk::ListItem>() {
                    li.set_child(Some(&label));
                }
            });
            factory.connect_bind(move |_, item| {
                let Some(li) = item.downcast_ref::<gtk::ListItem>() else { return };
                let (Some(label), Some(obj)) = (li.child().and_downcast::<gtk::Label>(), li.item().and_downcast::<glib::BoxedAnyObject>()) else { return };
                let mut row = obj.borrow_mut::<RowData>();
                set_cell(&label, row.cells.get(i).map(String::as_str).unwrap_or(""));
                row.labels.push((i, label.downgrade()));
            });
            factory.connect_unbind(move |_, item| {
                let Some(li) = item.downcast_ref::<gtk::ListItem>() else { return };
                let (Some(label), Some(obj)) = (li.child().and_downcast::<gtk::Label>(), li.item().and_downcast::<glib::BoxedAnyObject>()) else { return };
                obj.borrow_mut::<RowData>().labels.retain(|(_, w)| w.upgrade().is_some_and(|l| l != label));
            });
            let column = gtk::ColumnViewColumn::new(Some(c.title), Some(factory));
            column.set_resizable(true);
            column.set_expand(c.expand);
            // Makes the header clickable; actual ordering happens in `refresh`.
            column.set_sorter(Some(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal)));
            view.append_column(&column);
        }
        let selection = gtk::SingleSelection::new(Some(filtered));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        view.set_model(Some(&selection));

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let count = gtk::Label::new(None);
        count.add_css_class("dim-label");
        count.add_css_class("caption");
        count.set_xalign(0.0);
        if searchable {
            let search = gtk::SearchEntry::new();
            search.set_placeholder_text(Some("Filter rows…"));
            search.update_property(&[gtk::accessible::Property::Label("Filter table rows")]);
            let q3 = query.clone();
            search.connect_search_changed(move |e| {
                *q3.borrow_mut() = e.text().to_lowercase();
                filter.changed(gtk::FilterChange::Different);
            });
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            search.set_hexpand(true);
            bar.append(&search);
            bar.append(&count);
            widget.append(&bar);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&view));
        scroll.set_min_content_height(height);
        scroll.set_vexpand(true);
        scroll.set_propagate_natural_height(height <= 0);
        scroll.add_css_class("card");
        widget.append(&scroll);
        let t = Rc::new(DataTable { widget, view, store, selection, key_col, count, numeric: cols.iter().map(|c| c.numeric).collect(), rows: RefCell::default(), objects: RefCell::default() });
        if let Some(sorter) = t.view.sorter() {
            let weak = Rc::downgrade(&t);
            sorter.connect_changed(move |_, _| {
                if let Some(t) = weak.upgrade() {
                    t.refresh();
                }
            });
        }
        t
    }

    pub fn set_rows(&self, rows: Vec<Vec<String>>) {
        if *self.rows.borrow() == rows {
            return;
        }
        *self.rows.borrow_mut() = rows;
        self.refresh();
    }

    /// Current header sort: (column index, descending).
    fn sort_state(&self) -> Option<(usize, bool)> {
        let sorter = self.view.sorter().and_downcast::<gtk::ColumnViewSorter>()?;
        let col = sorter.primary_sort_column()?;
        let cols = self.view.columns();
        let idx = (0..cols.n_items()).find(|i| cols.item(*i).as_ref() == Some(col.upcast_ref()))? as usize;
        Some((idx, sorter.primary_sort_order() == gtk::SortType::Descending))
    }

    fn refresh(&self) {
        let rows = self.rows.borrow();
        let mut order: Vec<usize> = (0..rows.len()).collect();
        if let Some((c, desc)) = self.sort_state() {
            if self.numeric.get(c).copied().unwrap_or(false) {
                let keys: Vec<f64> = rows.iter().map(|r| r.get(c).map_or(f64::NEG_INFINITY, |v| sort_key(v))).collect();
                order.sort_by(|a, b| keys[*a].total_cmp(&keys[*b]));
            } else {
                let keys: Vec<String> = rows.iter().map(|r| r.get(c).map(|v| v.to_lowercase()).unwrap_or_default()).collect();
                order.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
            }
            if desc {
                order.reverse();
            }
        }
        // Reuse row objects by key; update their visible labels in place.
        let mut old = std::mem::take(&mut *self.objects.borrow_mut());
        let mut next = std::collections::HashMap::with_capacity(rows.len());
        let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut objs = Vec::with_capacity(rows.len());
        for &i in &order {
            let r = &rows[i];
            let k = r.get(self.key_col).map(String::as_str).unwrap_or("");
            let n = seen.entry(k).or_insert(0);
            let key = format!("{k}\u{0}{n}");
            *n += 1;
            let obj = match old.remove(&key) {
                Some(o) => {
                    let mut d = o.borrow_mut::<RowData>();
                    if d.cells != *r {
                        d.cells.clone_from(r);
                        d.labels.retain(|(_, w)| w.upgrade().is_some());
                        for (col, w) in &d.labels {
                            if let Some(l) = w.upgrade() {
                                set_cell(&l, r.get(*col).map(String::as_str).unwrap_or(""));
                            }
                        }
                    }
                    drop(d);
                    o
                }
                None => glib::BoxedAnyObject::new(RowData { cells: r.clone(), labels: Vec::new() }),
            };
            next.insert(key, obj.clone());
            objs.push(obj);
        }
        drop(rows);
        *self.objects.borrow_mut() = next;
        let same_order = self.store.n_items() as usize == objs.len()
            && objs.iter().enumerate().all(|(i, o)| self.store.item(i as u32).is_some_and(|x| x.as_ptr() == o.upcast_ref::<glib::Object>().as_ptr()));
        if !same_order {
            let key = self.selected().and_then(|r| r.get(self.key_col).cloned());
            self.store.splice(0, self.store.n_items(), &objs);
            if let (Some(k), Some(m)) = (key, self.selection.model()) {
                if let Some(i) = (0..m.n_items()).find(|i| m.item(*i).is_some_and(|o| cells(&o).get(self.key_col) == Some(&k))) {
                    self.selection.set_selected(i);
                }
            }
        }
        self.count.set_text(&format!("{} rows", objs.len()));
    }

    pub fn selected(&self) -> Option<Vec<String>> {
        self.selection.selected_item().map(|o| cells(&o))
    }

    /// Sort by column index (descending when `desc`).
    pub fn sort_by(&self, col: usize, desc: bool) {
        let cols = self.view.columns();
        if let Some(c) = cols.item(col as u32).and_downcast::<gtk::ColumnViewColumn>() {
            self.view.sort_by_column(Some(&c), if desc { gtk::SortType::Descending } else { gtk::SortType::Ascending });
        }
    }

    pub fn connect_activate(&self, f: impl Fn(Vec<String>) + 'static) {
        let sel = self.selection.clone();
        self.view.connect_activate(move |_, pos| {
            if let Some(o) = sel.item(pos) {
                f(cells(&o));
            }
        });
    }
}

fn set_cell(label: &gtk::Label, text: &str) {
    if label.text() != text {
        label.set_text(text);
        label.set_tooltip_text((text.len() > 40).then_some(text));
    }
}
