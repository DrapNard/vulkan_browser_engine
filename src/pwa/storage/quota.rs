use std::collections::HashMap;

pub struct QuotaManager {
    origin_quotas: HashMap<String, OriginQuota>,
    global_quota: u64,
    used_quota: u64,
}

pub struct OriginQuota {
    origin: String,
    quota: u64,
    used: u64,
    persistent: bool,
}

impl QuotaManager {
    pub fn new() -> Self {
        Self {
            origin_quotas: HashMap::new(),
            global_quota: 10 * 1024 * 1024 * 1024, // 10GB
            used_quota: 0,
        }
    }

    pub async fn request_quota(&mut self, origin: &str, requested: u64) -> Result<u64, QuotaError> {
        let available = self.global_quota - self.used_quota;
        let granted = requested.min(available);

        let quota = self
            .origin_quotas
            .entry(origin.to_string())
            .or_insert_with(|| OriginQuota {
                origin: origin.to_string(),
                quota: 0,
                used: 0,
                persistent: false,
            });

        quota.quota += granted;
        self.used_quota += granted;

        Ok(granted)
    }

    pub async fn check_quota(&self, origin: &str, size: u64) -> bool {
        if let Some(quota) = self.origin_quotas.get(origin) {
            quota.used + size <= quota.quota
        } else {
            size <= self.global_quota - self.used_quota
        }
    }

    pub async fn use_quota(&mut self, origin: &str, size: u64) -> Result<(), QuotaError> {
        if !self.check_quota(origin, size).await {
            return Err(QuotaError::QuotaExceeded);
        }

        let quota = self
            .origin_quotas
            .entry(origin.to_string())
            .or_insert_with(|| OriginQuota {
                origin: origin.to_string(),
                quota: size,
                used: 0,
                persistent: false,
            });

        quota.used += size;
        Ok(())
    }

    pub async fn release_quota(&mut self, origin: &str, size: u64) {
        if let Some(quota) = self.origin_quotas.get_mut(origin) {
            quota.used = quota.used.saturating_sub(size);
        }
    }

    pub async fn clear_origin_data(&mut self, origin: &str) -> Result<u64, QuotaError> {
        if let Some(quota) = self.origin_quotas.remove(origin) {
            self.used_quota = self.used_quota.saturating_sub(quota.used);
            Ok(quota.used)
        } else {
            Ok(0)
        }
    }

    pub async fn get_usage(&self, origin: &str) -> Option<QuotaUsage> {
        self.origin_quotas.get(origin).map(|quota| QuotaUsage {
            quota: quota.quota,
            used: quota.used,
            available: quota.quota - quota.used,
            persistent: quota.persistent,
        })
    }

    pub async fn request_persistent_storage(&mut self, origin: &str) -> Result<bool, QuotaError> {
        if let Some(quota) = self.origin_quotas.get_mut(origin) {
            quota.persistent = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn estimate_usage(&self) -> StorageEstimate {
        StorageEstimate {
            quota: self.global_quota,
            usage: self.used_quota,
            usage_details: self
                .origin_quotas
                .iter()
                .map(|(origin, quota)| (origin.clone(), quota.used))
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuotaUsage {
    pub quota: u64,
    pub used: u64,
    pub available: u64,
    pub persistent: bool,
}

#[derive(Debug, Clone)]
pub struct StorageEstimate {
    pub quota: u64,
    pub usage: u64,
    pub usage_details: HashMap<String, u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    #[error("Quota exceeded")]
    QuotaExceeded,
    #[error("Permission denied")]
    PermissionDenied,
    #[error("Invalid request")]
    InvalidRequest,
}

impl Default for QuotaManager {
    fn default() -> Self {
        Self::new()
    }
}
