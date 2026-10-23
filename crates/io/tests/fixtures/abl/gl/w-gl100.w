/* gl/w-gl100.w: window part of the gl100 screen. */
TITLE = "<insert SmartWindow title>".
ASSIGN vfolder = vfolder + "|Cumuls".
RUN set-attribute-list IN h_folder ('FOLDER-LABELS = ':U + vfolder + ',').
DEFINE MENU POPUP-MENU-gl100
    MENU-ITEM m_masquer LABEL "&Masquer".
{&FRAME-NAME}:TITLE = "gl100 - Grand livre".
