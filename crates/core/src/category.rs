//! Rule-based spending categorization from the merchant string.
//!
//! Labels are Spanish and stable (stored in `transactions.category`). Rules are
//! ordered — first match wins. Tuned to the user's real merchants; extend freely.

/// Classify a transaction into a category. Incoming money is always "Ingreso".
pub fn categorize(merchant: &str, direction: &str) -> &'static str {
    if direction.eq_ignore_ascii_case("in") {
        return "Ingreso";
    }
    let m = merchant.to_uppercase();
    let has = |kws: &[&str]| kws.iter().any(|k| m.contains(k));

    if has(&["UBER", "RIDES", "CABIFY", "DIDI", "TAXI", "BEAT"]) {
        "Transporte"
    } else if has(&[
        "TAMBO", "PUKU", "RAPPI", "PEDIDOSYA", "KFC", "BEMBOS", "STARBUCKS", "PIZZA", "REST",
        "VALDEZ", "MCDONALD", "POPEYES", "NORKY", "DELIVERY", "MASS", "CAFE", "SANGUCH",
    ]) {
        "Comida"
    } else if has(&[
        "MAKRO", "PLAZA VEA", "WONG", "METRO", "TOTTUS", "VIVANDA", "OECHSLE", "RIPLEY",
        "FALABELLA", "SODIMAC", "PROMART",
    ]) {
        "Compras"
    } else if has(&["SMART FIT", "GYM", "INKAFARMA", "MIFARMA", "BOTICA", "FARMACIA", "CLINICA", "SANNA"]) {
        "Salud"
    } else if has(&[
        "NETFLIX", "SPOTIFY", "CLARO", "MOVISTAR", "ENTEL", "RECIBO", "SEDAPAL", "OSINERG",
        "GOOGLE", "APPLE", "AMAZON", "OPENAI", "ANTHROPIC",
    ]) {
        "Servicios"
    } else if has(&["PLIN", "YAPE"]) {
        "Transferencia"
    } else {
        "Otros"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_real_merchants() {
        assert_eq!(categorize("DLC*UBER RIDES", "out"), "Transporte");
        assert_eq!(categorize("DLC*RIDES", "out"), "Transporte");
        assert_eq!(categorize("TIENDAS TAMBO SAC", "out"), "Comida");
        assert_eq!(categorize("PUKU PUKU EL POLO", "out"), "Comida");
        assert_eq!(categorize("359 MAKRO SANTA ANITA", "out"), "Compras");
        assert_eq!(categorize("Smart Fit Peru", "out"), "Salud");
        assert_eq!(categorize("PLIN-GUSTAVO YNJANTE", "out"), "Transferencia");
        assert_eq!(categorize("Openpay", "out"), "Otros");
    }

    #[test]
    fn income_is_ingreso() {
        assert_eq!(categorize("Scale Labs", "in"), "Ingreso");
        assert_eq!(categorize("cualquier cosa", "in"), "Ingreso");
    }
}
