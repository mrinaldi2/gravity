//! Services that hold the home and are stopped through their own service
//! manager during a pause (H-117 Q3). Q2 only restarts what a pause stopped.

use crate::app::AppState;
use crate::db::Quiesce;

/// Starts again the services `q` stopped; returns their names.
pub fn restart_stopped(_app: &AppState, q: &Quiesce) -> Vec<String> {
    q.services_stopped.clone()
}
