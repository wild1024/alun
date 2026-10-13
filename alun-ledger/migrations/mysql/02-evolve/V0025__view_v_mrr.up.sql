CREATE VIEW v_mrr AS SELECT tenant_id, DATE_FORMAT(created_at, '%Y-%m') AS month, total, status FROM invoices WHERE status = 'paid'
