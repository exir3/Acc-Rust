//! Printable report metadata types.

use serde::Serialize;

use crate::report::layout::html;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintDoc {
    pub html: String,
    pub title: String,
    pub subtitle: String,
    pub file_name: String,
    pub paper_width_mm: f64,
    pub paper_height_mm: f64,
    pub margin_top_mm: f64,
    pub margin_right_mm: f64,
    pub margin_bottom_mm: f64,
    pub margin_left_mm: f64,
}

#[derive(Debug, Clone)]
pub struct ReportMeta {
    pub company: String,
    pub fiscal_year: String,
    pub today: String,
    /// Set by the trial-balance print command when the user unchecks
    /// "نمایش ساعت و تاریخ چاپ".
    pub show_print_date_time: bool,
}

impl ReportMeta {
    pub(crate) fn company(&self) -> String {
        let value = self.company.trim();
        if value.is_empty() {
            "واحد اقتصادی".to_string()
        } else {
            html(value)
        }
    }

    pub(crate) fn year(&self) -> String {
        let value = self.fiscal_year.trim();
        if value.is_empty() {
            "—".to_string()
        } else {
            html(value)
        }
    }
}
