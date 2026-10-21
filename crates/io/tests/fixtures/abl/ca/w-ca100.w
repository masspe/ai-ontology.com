/* ca/w-ca100.w: window part of the ca100 screen. */
TITLE = "<insert SmartWindow title>".
ASSIGN vfolder = vfolder + "|Cumuls".
RUN set-attribute-list IN h_folder ('FOLDER-LABELS = ':U + vfolder + ',').
DEFINE MENU POPUP-MENU-ca100
    MENU-ITEM m_masquer LABEL "&Masquer".
{&FRAME-NAME}:TITLE = "ca100 - Comptabilite analytique".
