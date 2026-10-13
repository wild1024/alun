CREATE UNIQUE INDEX uq_subscriptions_period ON subscriptions (tenant_id, customer_id, period_start)
