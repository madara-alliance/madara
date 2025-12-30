module.exports = {
  async up(db) {
    // Create indexes for the 'jobs' collection
    const jobs = db.collection("jobs");
    await jobs.createIndex({ id: 1 });
    await jobs.createIndex({ job_type: 1, internal_id: -1 }, { unique: true });
    // This compound index also serves queries on { job_type, status } via index prefix
    await jobs.createIndex({ job_type: 1, status: 1, internal_id: -1 });
    await jobs.createIndex({ status: 1 });
  },

  async down(db) {
    // Drop indexes for the 'jobs' collection
    await db.collection("jobs").dropIndex("id_1");
    await db.collection("jobs").dropIndex("job_type_1_internal_id_-1");
    await db.collection("jobs").dropIndex("job_type_1_status_1_internal_id_-1");
    await db.collection("jobs").dropIndex("status_1");
  },
};
