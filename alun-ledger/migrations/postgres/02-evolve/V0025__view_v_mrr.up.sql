CREATE VIEW v_mrr AS SELECT tenant_id, to_char(created_at, 'YYYY-MM') AS month, total, status FROM invoices WHERE status = 'paid'
