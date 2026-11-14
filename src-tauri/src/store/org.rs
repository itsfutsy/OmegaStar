//! Employees and their schedules, people, and hire proposals.

use super::*;

impl Store {
    // ---- employees ----

    /// Everyone on the team, oldest hire first. The front desk is not on the team, so it is left out: see `desk`.
    pub fn employees(&self) -> Result<Vec<Employee>> {
        let mut v = self.everyone()?;
        v.retain(|e| !e.desk);
        Ok(v)
    }

    /// The front desk, once it has been set up.
    pub fn desk(&self) -> Result<Option<Employee>> {
        Ok(self.everyone()?.into_iter().find(|e| e.desk))
    }

    fn everyone(&self) -> Result<Vec<Employee>> {
        let mut st = self.conn.prepare("SELECT data FROM employees WHERE departed_at IS NULL")?;
        let mut v: Vec<Employee> = readable("employees", st.query_map([], |r| from_row(r, 0))?)?;
        v.sort_by_key(|e| e.hired_at);
        Ok(v)
    }

    pub fn employee(&self, id: &str) -> Result<Option<Employee>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM employees WHERE id = ? AND departed_at IS NULL", [id], |r| from_row(r, 0))
            .optional()?)
    }

    /// Every slug ever used, departed employees included (the column is UNIQUE).
    pub fn slugs(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT slug FROM employees")?;
        let v = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(v)
    }

    pub fn save_employee(&self, e: &Employee) -> Result<()> {
        self.conn.execute(
            "INSERT INTO employees(id, slug, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET data = ?3",
            params![e.id, e.slug, to_json(e)?],
        )?;
        Ok(())
    }

    pub fn depart_employee(&self, id: &str, at: Ts) -> Result<()> {
        self.conn.execute("UPDATE employees SET departed_at = ? WHERE id = ?", params![ms(at), id])?;
        Ok(())
    }

    // ---- people ----

    pub fn people(&self) -> Result<Vec<Person>> {
        let mut st = self.conn.prepare("SELECT data FROM people WHERE removed_at IS NULL")?;
        let mut v: Vec<Person> = readable("people", st.query_map([], |r| from_row(r, 0))?)?;
        v.sort_by_key(|p| p.added_at);
        Ok(v)
    }

    pub fn person(&self, id: &str) -> Result<Option<Person>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM people WHERE id = ? AND removed_at IS NULL", [id], |r| from_row(r, 0))
            .optional()?)
    }

    pub fn save_person(&self, p: &Person) -> Result<()> {
        self.conn.execute(
            "INSERT INTO people(id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data = ?2",
            params![p.id, to_json(p)?],
        )?;
        Ok(())
    }

    pub fn remove_person(&self, id: &str, at: Ts) -> Result<()> {
        self.conn.execute("UPDATE people SET removed_at = ? WHERE id = ?", params![ms(at), id])?;
        Ok(())
    }

    // ---- schedules ----

    fn schedules_where(&self, clause: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<(Schedule, Ts)>> {
        let mut st = self.conn.prepare(&format!("SELECT data, last_slot FROM schedules {clause}"))?;
        let rows = st.query_map(p, |r| {
            let s: Schedule = from_row(r, 0)?;
            let last: i64 = r.get(1)?;
            Ok((s, Ts::from_timestamp_millis(last).unwrap_or_default()))
        })?;
        let v = readable("schedules", rows)?;
        Ok(v)
    }

    /// Schedules with the slot they last fired (or were anchored) at.
    pub fn all_schedules(&self) -> Result<Vec<(Schedule, Ts)>> {
        self.schedules_where("", &[])
    }

    pub fn schedules(&self, employee_id: &str) -> Result<Vec<Schedule>> {
        Ok(self.schedules_where("WHERE employee_id = ?", &[&employee_id])?.into_iter().map(|(s, _)| s).collect())
    }

    pub fn schedule(&self, id: &str) -> Result<Option<(Schedule, Ts)>> {
        Ok(self.schedules_where("WHERE id = ?", &[&id])?.into_iter().next())
    }

    pub fn save_schedule(&self, s: &Schedule, last_slot: Ts) -> Result<()> {
        self.conn.execute(
            "INSERT INTO schedules(id, employee_id, last_slot, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET last_slot = ?3, data = ?4",
            params![s.id, s.employee_id, ms(last_slot), to_json(s)?],
        )?;
        Ok(())
    }

    pub fn set_last_slot(&self, schedule_id: &str, slot: Ts) -> Result<()> {
        self.conn.execute("UPDATE schedules SET last_slot = ? WHERE id = ?", params![ms(slot), schedule_id])?;
        Ok(())
    }

    pub fn delete_schedule(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM schedules WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn delete_schedules_of(&self, employee_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM schedules WHERE employee_id = ?", [employee_id])?;
        Ok(())
    }

    // ---- hire proposals ----

    pub fn save_proposal(&self, p: &HireProposal) -> Result<()> {
        self.conn.execute(
            "INSERT INTO proposals(id, employee_id, state, created_at, data) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET state = ?3, data = ?5",
            params![p.id, p.employee_id, p.state.as_str(), ms(p.created_at), to_json(p)?],
        )?;
        Ok(())
    }

    pub fn proposal(&self, id: &str) -> Result<Option<HireProposal>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM proposals WHERE id = ?", [id], |r| from_row(r, 0))
            .optional()?)
    }

    /// Newest first.
    pub fn proposals(&self, employee_id: Option<&str>, state: Option<ProposalState>) -> Result<Vec<HireProposal>> {
        let mut st = self.conn.prepare(
            "SELECT data FROM proposals WHERE (?1 IS NULL OR employee_id = ?1) AND (?2 IS NULL OR state = ?2)
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let v = readable("proposals", st.query_map(params![employee_id, state.map(|s| s.as_str())], |r| from_row(r, 0))?)?;
        Ok(v)
    }
}
