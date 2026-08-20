//! Database access.
//!
//! Every tenant-scoped query in [`repo`] takes an `org_id` and includes it in the WHERE
//! clause. That is the tenant isolation boundary, and it is enforced by tests that
//! attempt cross-tenant reads and require them to fail.

pub mod pool;
pub mod repo;
