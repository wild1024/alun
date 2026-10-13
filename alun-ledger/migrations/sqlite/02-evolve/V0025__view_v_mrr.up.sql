CREATE VIEW v_mrr AS SELECT tenant_id, substr(created_at, 1, 7) AS month, total, status FROM invoices WHERE status = 'paid'
