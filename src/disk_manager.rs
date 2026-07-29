use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const PAGE_SIZE: usize = 4096;
pub type PageId = u32;

pub struct DiskManager {
    file: File,
    next_page_id: PageId,
}

impl DiskManager {
    /// Opens the database file at the given path. Creates it if it doesn't exist.
    /// Initializes Page 0 (Metadata Page) for the Free List if the file is brand new.
    pub fn new(file_path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path)?;

        let file_size = file.metadata()?.len();
        let next_page_id = if file_size == 0 {
            let zero_page = [0u8; PAGE_SIZE];
            file.write_all(&zero_page)?;
            file.sync_all()?;
            1
        } else {
            (file_size / PAGE_SIZE as u64) as PageId
        };

        Ok(Self { file, next_page_id })
    }

    /// Reads exactly 4KB of data from disk into the provided `page_data` buffer.
    pub fn read_page(
        &mut self,
        page_id: PageId,
        page_data: &mut [u8; PAGE_SIZE],
    ) -> io::Result<()> {
        let offset = (page_id as usize * PAGE_SIZE) as u64;
        self.file.seek(SeekFrom::Start(offset))?;

        self.file.read_exact(page_data)?;

        Ok(())
    }

    /// Writes exactly 4KB of data from `page_data` to the disk.
    pub fn write_page(&mut self, page_id: PageId, page_data: &[u8; PAGE_SIZE]) -> io::Result<()> {
        let offset = (page_id as usize * PAGE_SIZE) as u64;

        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(page_data)?;
        self.file.sync_all()?;

        Ok(())
    }

    /// Allocates a logical page. Pops the head of the free list if available.
    pub fn allocate_page(&mut self) -> PageId {
        let mut meta_page = [0u8; PAGE_SIZE];
        self.read_page(0, &mut meta_page)
            .expect("Failed to read metadata page");

        let head_page_id = u32::from_le_bytes(meta_page[0..4].try_into().unwrap());

        if head_page_id != 0 {
            let recycled_id = head_page_id;

            let mut recycled_page = [0u8; PAGE_SIZE];
            self.read_page(recycled_id, &mut recycled_page)
                .expect("Failed to read recycled page");
            let next_head_id = u32::from_le_bytes(recycled_page[0..4].try_into().unwrap());

            meta_page[0..4].copy_from_slice(&next_head_id.to_le_bytes());
            self.write_page(0, &meta_page)
                .expect("Failed to update metadata page");

            return recycled_id;
        }

        let page_id = self.next_page_id;
        self.next_page_id += 1;

        let empty_page = [0u8; PAGE_SIZE];
        self.write_page(page_id, &empty_page)
            .expect("Failed to physically grow the file");

        page_id
    }

    /// Deallocates a page by pushing it to the front of the free list.
    pub fn deallocate_page(&mut self, page_id: PageId) {
        if page_id == 0 {
            panic!("Cannot deallocate the metadata page (Page 0)!");
        }

        let mut meta_page = [0u8; PAGE_SIZE];
        self.read_page(0, &mut meta_page)
            .expect("Failed to read metadata page");
        let current_head = u32::from_le_bytes(meta_page[0..4].try_into().unwrap());

        let mut empty_page = [0u8; PAGE_SIZE];
        empty_page[0..4].copy_from_slice(&current_head.to_le_bytes());
        self.write_page(page_id, &empty_page)
            .expect("Failed to write to deallocated page");

        meta_page[0..4].copy_from_slice(&page_id.to_le_bytes());
        self.write_page(0, &meta_page)
            .expect("Failed to update metadata page");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_allocate_page() {
        let test_file = "test_allocate.db";
        let _ = fs::remove_file(test_file);

        let mut disk_manager = DiskManager::new(test_file).unwrap();

        assert_eq!(disk_manager.allocate_page(), 1);
        assert_eq!(disk_manager.allocate_page(), 2);
        assert_eq!(disk_manager.allocate_page(), 3);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_read_write_page() {
        let test_file = "test_read_write.db";
        let _ = fs::remove_file(test_file);

        let mut disk_manager = DiskManager::new(test_file).unwrap();
        let page_id = disk_manager.allocate_page();

        let write_page = [65u8; PAGE_SIZE];
        disk_manager.write_page(page_id, &write_page).unwrap();

        let mut read_page = [0u8; PAGE_SIZE];
        disk_manager.read_page(page_id, &mut read_page).unwrap();

        assert_eq!(write_page, read_page);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_page_recycling() {
        let test_file = "test_recycling.db";
        let _ = fs::remove_file(test_file);

        {
            let mut disk_manager = DiskManager::new(test_file).unwrap();

            assert_eq!(disk_manager.allocate_page(), 1);
            assert_eq!(disk_manager.allocate_page(), 2);
            assert_eq!(disk_manager.allocate_page(), 3);

            disk_manager.deallocate_page(2);
        }

        {
            let mut disk_manager = DiskManager::new(test_file).unwrap();

            assert_eq!(disk_manager.allocate_page(), 2);

            assert_eq!(disk_manager.allocate_page(), 4);
        }

        let _ = fs::remove_file(test_file);
    }
}
