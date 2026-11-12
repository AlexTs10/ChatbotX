//! Port of `packages/utils/__tests__/sanitize-file-name.test.ts`.

use chatbotx_core::utils::sanitize_file_name::sanitize_file_name;

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:5 "sanitizeFileName > "CMND mặt trước.JPG" becomes "cmnd-mat-truoc.jpg""
#[test]
fn sanitizefilename_cmnd_m_t_tr_c_jpg_becomes_cmnd_mat_truoc_jpg() {
    assert_eq!(
        sanitize_file_name("CMND mặt trước.JPG"),
        "cmnd-mat-truoc.jpg"
    );
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:5 "sanitizeFileName > "Đơn đăng ký (1).pdf" becomes "don-dang-ky-1.pdf""
#[test]
fn sanitizefilename_n_ng_k_1_pdf_becomes_don_dang_ky_1_pdf() {
    assert_eq!(
        sanitize_file_name("Đơn đăng ký (1).pdf"),
        "don-dang-ky-1.pdf"
    );
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:5 "sanitizeFileName > "../../etc/passwd" becomes "etc-passwd""
#[test]
fn sanitizefilename_etc_passwd_becomes_etc_passwd() {
    assert_eq!(sanitize_file_name("../../etc/passwd"), "etc-passwd");
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:5 "sanitizeFileName > "  spaced   name .pdf" becomes "spaced-name.pdf""
#[test]
fn sanitizefilename_spaced_name_pdf_becomes_spaced_name_pdf() {
    assert_eq!(
        sanitize_file_name("  spaced   name .pdf"),
        "spaced-name.pdf"
    );
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:5 "sanitizeFileName > "mix--ed__name..PDF" becomes "mix-ed__name.pdf""
#[test]
fn sanitizefilename_mix_ed_name_pdf_becomes_mix_ed_name_pdf() {
    assert_eq!(sanitize_file_name("mix--ed__name..PDF"), "mix-ed__name.pdf");
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:15 "sanitizeFileName > a name with nothing usable left ("###.png") becomes "file.png""
#[test]
fn sanitizefilename_a_name_with_nothing_usable_left_png_becomes_file_png() {
    assert_eq!(sanitize_file_name("###.png"), "file.png");
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:15 "sanitizeFileName > a name with nothing usable left ("名前.pdf") becomes "file.pdf""
#[test]
fn sanitizefilename_a_name_with_nothing_usable_left_pdf_becomes_file_pdf() {
    assert_eq!(sanitize_file_name("名前.pdf"), "file.pdf");
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:15 "sanitizeFileName > a name with nothing usable left ("") becomes "file""
#[test]
fn sanitizefilename_a_name_with_nothing_usable_left_becomes_file() {
    assert_eq!(sanitize_file_name(""), "file");
}

/// Port of: packages/utils/__tests__/sanitize-file-name.test.ts:23 "sanitizeFileName > keeps at most 80 characters, extension included, without a dangling dash"
#[test]
fn sanitizefilename_keeps_at_most_80_characters_extension_included_without_a_dangling_dash() {
    // The cut lands right after "a…a-", so the dash it leaves is dropped.
    let name = sanitize_file_name(&format!("{} b c d.jpeg", "a".repeat(74)));

    assert_eq!(name, format!("{}.jpeg", "a".repeat(74)));
    assert_eq!(
        sanitize_file_name(&format!("{}.jpeg", "x".repeat(200))),
        format!("{}.jpeg", "x".repeat(75))
    );
}
