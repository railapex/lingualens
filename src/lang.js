// Language detection and Spanish accent normalization

// Common Spanish words that lose accents in casual typing.
// ONLY includes words where the accented form is overwhelmingly more common,
// or the unaccented form isn't a real word.
// Excludes ambiguous pairs: como/cómo, esta/está, que/qué, donde/dónde, etc.
const ACCENT_MAP = {
  // Unambiguous nouns/greetings
  'dias': 'días', 'dia': 'día',
  'adios': 'adiós',
  'cafe': 'café',
  'musica': 'música',
  'telefono': 'teléfono',
  'numero': 'número',
  'pagina': 'página',
  'articulo': 'artículo',
  'sabado': 'sábado', 'miercoles': 'miércoles',
  'tambien': 'también',
  'aqui': 'aquí', 'ahi': 'ahí', 'alla': 'allá',
  'detras': 'detrás',
  'facil': 'fácil', 'dificil': 'difícil',
  'rapido': 'rápido',
  'ultimo': 'último',
  'clasico': 'clásico',
  'informacion': 'información',
  'educacion': 'educación',
  'habitacion': 'habitación',
  'situacion': 'situación',
  'razon': 'razón', 'corazon': 'corazón',
  'cancion': 'canción',
  'jardin': 'jardín',
  'ingles': 'inglés', 'frances': 'francés',
  'bebe': 'bebé',
  // -ía verb forms (unambiguous — unaccented forms aren't real words)
  'podria': 'podría',
  'queria': 'quería',
  'tenia': 'tenía',
  'sabia': 'sabía',
  'decia': 'decía',
  'haria': 'haría',
  'vendria': 'vendría',
  'comere': 'comeré',
  'sera': 'será',
};

// Interrogative words — only accent when preceded by ¿ or at start of question
const INTERROGATIVES = {
  'como': 'cómo', 'donde': 'dónde', 'cuando': 'cuándo',
  'que': 'qué', 'quien': 'quién', 'cual': 'cuál',
  'cuanto': 'cuánto', 'cuantos': 'cuántos',
};

/**
 * Normalize missing accents in Spanish text.
 * Conservative — only fixes high-confidence cases.
 */
export function normalizeAccents(text) {
  const isQuestion = text.includes('¿') || text.endsWith('?');

  return text.replace(/\b\w+\b/g, (word) => {
    const lower = word.toLowerCase();

    // Check unambiguous replacements first
    let replacement = ACCENT_MAP[lower];

    // Interrogatives only in question context
    if (!replacement && isQuestion) {
      replacement = INTERROGATIVES[lower];
    }

    if (!replacement) return word;

    // Preserve original casing of first letter
    if (word[0] === word[0].toUpperCase()) {
      return replacement[0].toUpperCase() + replacement.slice(1);
    }
    return replacement;
  });
}

/**
 * Detect language — simple heuristic.
 * Spanish indicators: accented chars, ñ, inverted punctuation.
 * Also checks for common Spanish words as fallback.
 */
const SPANISH_WORDS = new Set([
  'el', 'la', 'los', 'las', 'un', 'una', 'unos', 'unas',
  'de', 'del', 'en', 'con', 'por', 'para', 'sin',
  'es', 'son', 'ser', 'estar', 'hay', 'tiene',
  'que', 'como', 'donde', 'cuando', 'quien',
  'muy', 'bien', 'mal', 'hoy', 'ayer',
  'yo', 'tu', 'nosotros', 'ellos', 'ella',
  'hola', 'buenos', 'buenas', 'gracias',
  'si', 'no', 'pero', 'porque', 'aunque',
]);

export function detectLang(text) {
  // Strong signals: Spanish-specific characters
  if (/[áéíóúñ¿¡ü]/i.test(text)) return 'es';

  // Weak signal: common Spanish words
  const words = text.toLowerCase().split(/\s+/);
  const spanishCount = words.filter(w => SPANISH_WORDS.has(w)).length;
  if (spanishCount >= 2 || (words.length <= 3 && spanishCount >= 1)) return 'es';

  return 'en';
}
