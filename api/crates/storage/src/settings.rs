//! Where uploads are kept and how large they may be, as plain values with the
//! defaults the instance ships with. Reading them from the environment is the
//! caller's business.

/// Where uploads are kept.
#[derive(Clone, Debug, Default)]
pub struct StorageOptions {
    /// Files on the data volume, unless an S3-compatible store is named; the
    /// fields below are that store's.
    pub provider: StorageProvider,
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StorageProvider {
    /// Plain files in `files/` of the data volume.
    #[default]
    FileSystem,
    /// One bucket of an S3-compatible store.
    S3,
}

/// Size limits and quotas that apply to uploaded media.
#[derive(Clone, Debug)]
pub struct UploadOptions {
    pub max_image_bytes: i64,
    pub max_audio_bytes: i64,
    pub max_video_bytes: i64,
    pub max_file_bytes: i64,
    /// What one account may keep stored. Zero or less disables the quota.
    pub max_stored_bytes_per_user: i64,
    /// What the whole instance may keep stored. Zero or less disables the quota.
    pub max_stored_bytes: i64,
    /// Free space an upload must leave on the data volume, which the database
    /// and the logs share. Zero turns the check off.
    pub minimum_free_data_root_bytes: i64,
    /// Uploads handled at once; [`UploadOptions::concurrent_uploads`] clamps it.
    pub max_concurrent_uploads: i32,
}

impl Default for UploadOptions {
    fn default() -> Self {
        const MIB: i64 = 1024 * 1024;
        const GIB: i64 = 1024 * MIB;
        Self {
            max_image_bytes: 10 * MIB,
            max_audio_bytes: 25 * MIB,
            max_video_bytes: 100 * MIB,
            max_file_bytes: 25 * MIB,
            max_stored_bytes_per_user: 2 * GIB,
            max_stored_bytes: 20 * GIB,
            minimum_free_data_root_bytes: GIB,
            max_concurrent_uploads: 4,
        }
    }
}

impl UploadOptions {
    /// How many uploads run at once: the setting, kept between 1 and 32.
    pub fn concurrent_uploads(&self) -> usize {
        self.max_concurrent_uploads.clamp(1, 32) as usize
    }
}
