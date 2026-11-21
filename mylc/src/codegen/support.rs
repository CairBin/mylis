use super::*;

impl<'a> Codegen<'a> {
    pub(super) fn line(&mut self, s: &str) {
        if self.in_body {
            for _ in 0..self.ind {
                self.body.push_str("    ");
            }
            self.body.push_str(s);
            self.body.push('\n');
        } else {
            for _ in 0..self.ind {
                self.out.push_str("    ");
            }
            self.out.push_str(s);
            self.out.push('\n');
        }
    }

    pub(super) fn blank(&mut self) {
        if self.in_body {
            self.body.push('\n');
        } else {
            self.out.push('\n');
        }
    }

    pub(super) fn tmp(&mut self, hint: &str) -> String {
        self.tmp_counter += 1;
        format!("t_{}_{}", hint, self.tmp_counter)
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(super) fn declare_local(&mut self, src: &str, cname: &str) {
        if let Some(top) = self.scopes.last_mut() {
            top.push((src.to_string(), cname.to_string()));
        }
    }

    pub(super) fn lookup_local(&self, src: &str) -> Option<String> {
        for s in self.scopes.iter().rev() {
            if let Some((_, c)) = s.iter().rev().find(|(n, _)| n == src) {
                return Some(c.clone());
            }
        }
        None
    }
}
