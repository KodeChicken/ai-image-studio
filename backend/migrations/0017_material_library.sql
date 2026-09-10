CREATE TABLE material_categories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(64) NOT NULL UNIQUE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (sort_order >= 0)
);

CREATE TABLE material_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    category_id UUID NOT NULL REFERENCES material_categories(id) ON DELETE RESTRICT,
    title VARCHAR(128) NOT NULL,
    image_asset_id UUID REFERENCES image_assets(id) ON DELETE RESTRICT,
    image_url TEXT,
    source_url TEXT,
    source_author VARCHAR(256),
    license VARCHAR(128),
    published BOOLEAN NOT NULL DEFAULT FALSE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((image_asset_id IS NULL) <> (image_url IS NULL)),
    CHECK (sort_order >= 0)
);

CREATE INDEX ix_material_templates_public
    ON material_templates(category_id, sort_order, created_at)
    WHERE published;

CREATE INDEX ix_material_templates_image_asset_id
    ON material_templates(image_asset_id)
    WHERE image_asset_id IS NOT NULL;

INSERT INTO material_categories (id, name, sort_order) VALUES
    ('20000000-0000-4000-8000-000000000001', '电商产品', 0),
    ('20000000-0000-4000-8000-000000000002', '餐饮美食', 1),
    ('20000000-0000-4000-8000-000000000003', '美妆护肤', 2),
    ('20000000-0000-4000-8000-000000000004', '教育培训', 3),
    ('20000000-0000-4000-8000-000000000005', '旅游出行', 4),
    ('20000000-0000-4000-8000-000000000006', '节日活动', 5),
    ('20000000-0000-4000-8000-000000000007', '海报设计', 6);

INSERT INTO material_templates (
    id, category_id, title, image_url, source_url, source_author, license, published, sort_order
) VALUES
    ('21000000-0000-4000-8000-000000000001', '20000000-0000-4000-8000-000000000001', '新品登场', '/materials/ecommerce-01.webp', 'https://commons.wikimedia.org/wiki/File:Perfume_Bottle_MET_DP242904.jpg', 'Wikimedia Commons contributor', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000002', '20000000-0000-4000-8000-000000000001', '纯净护肤', '/materials/ecommerce-02.webp', 'https://commons.wikimedia.org/wiki/File:Korean_cosmetic_products.jpg', 'Jmh65890', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000003', '20000000-0000-4000-8000-000000000001', '咖啡时光', '/materials/ecommerce-03.webp', 'https://commons.wikimedia.org/wiki/File:Coffee_Beans,_color_(28183654506).jpg', 'Chuck Grimmett from Yonkers, NY, United States of America', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000004', '20000000-0000-4000-8000-000000000001', '悦享聆听', '/materials/ecommerce-04.webp', 'https://commons.wikimedia.org/wiki/File:Music_Headphones_(Unsplash).jpg', 'Angello Lopez angello_pro', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000005', '20000000-0000-4000-8000-000000000001', '步履新章', '/materials/ecommerce-05.webp', 'https://commons.wikimedia.org/wiki/File:Beach-sand-shoe-green-yellow-product-shoes-sneakers-footwear-clog-outdoor-shoe-1121290.jpg', 'Wikimedia Commons contributor', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000006', '20000000-0000-4000-8000-000000000001', '闪耀时刻', '/materials/ecommerce-06.webp', 'https://commons.wikimedia.org/wiki/File:Ring_MET_DP205040.jpg', 'Wikimedia Commons contributor', 'CC0 1.0', TRUE, 5),
    ('21000000-0000-4000-8000-000000000007', '20000000-0000-4000-8000-000000000002', '今日披萨', '/materials/food-01.webp', 'https://commons.wikimedia.org/wiki/File:Pepperoni_and_mushroom_pizza_-_Massachusetts.jpg', 'Daderot', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000008', '20000000-0000-4000-8000-000000000002', '一碗好面', '/materials/food-02.webp', 'https://commons.wikimedia.org/wiki/File:Bowl_of_Ramen_(Unsplash_1RyWImEc7K4).jpg', 'Lizette Carrasco lizivonne', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000009', '20000000-0000-4000-8000-000000000002', '城市咖啡', '/materials/food-03.webp', 'https://commons.wikimedia.org/wiki/File:Two_cups_of_iced_coffee_at_Coast_Cafe,_Worthing_2026-05-25.jpg', 'Andy Li', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000010', '20000000-0000-4000-8000-000000000002', '每日现烤', '/materials/food-04.webp', 'https://commons.wikimedia.org/wiki/File:SZ_%E6%B7%B1%E5%9C%B3_Shenzhen_%E5%8D%97%E5%B1%B1%E5%8D%80_Nanshan_%E6%B7%B1%E5%9C%B3%E7%81%A3_Shenzhen_Bay_MixC_mall_shop_1-7_Bread_Bakery_May_2026_N13P_02.jpg', 'TWS LUOK Mingaowpuo', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000011', '20000000-0000-4000-8000-000000000002', '轻食主义', '/materials/food-05.webp', 'https://commons.wikimedia.org/wiki/File:Fresh_Salad_(Unsplash).jpg', 'fireskystudios.com asthetik', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000012', '20000000-0000-4000-8000-000000000002', '甜蜜限定', '/materials/food-06.webp', 'https://commons.wikimedia.org/wiki/File:Piece_of_chocolate_cake_on_a_white_plate_decorated_with_chocolate_sauce.jpg', 'Daria YakovlevaMinor edits made by Subsidiary account', 'CC0 1.0', TRUE, 5),
    ('21000000-0000-4000-8000-000000000013', '20000000-0000-4000-8000-000000000003', '自然妆感', '/materials/beauty-01.webp', 'https://commons.wikimedia.org/wiki/File:Cosmetics-1078712_1280.jpg', 'camillaperrucci', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000014', '20000000-0000-4000-8000-000000000003', '香气成诗', '/materials/beauty-02.webp', 'https://commons.wikimedia.org/wiki/File:Scent_Bottle_SAAM-1929.8.245.47_1.jpg', 'Wikimedia Commons contributor', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000015', '20000000-0000-4000-8000-000000000003', '肌肤焕亮', '/materials/beauty-03.webp', 'https://commons.wikimedia.org/wiki/File:Cosmetics1.jpg', 'Ibrahim Sani Mustapha', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000016', '20000000-0000-4000-8000-000000000003', '精致妆容', '/materials/beauty-04.webp', 'https://commons.wikimedia.org/wiki/File:Makeup-brush-tips.jpg', 'Sarah Pflug', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000017', '20000000-0000-4000-8000-000000000003', '静享时光', '/materials/beauty-05.webp', 'https://commons.wikimedia.org/wiki/File:Candle_(Unsplash).jpg', 'Jeffrey Wegrzyn jeffreymwegrzyn', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000018', '20000000-0000-4000-8000-000000000003', '春日花漾', '/materials/beauty-06.webp', 'https://commons.wikimedia.org/wiki/File:Still_Life_of_Flowers_by_Adelheid_Dietrich,_1868,_oil_on_wood_-_National_Gallery_of_Art,_Washington_-_DSC00103.JPG', 'Adelheid Dietrich', 'CC0 1.0', TRUE, 5),
    ('21000000-0000-4000-8000-000000000019', '20000000-0000-4000-8000-000000000004', '阅读计划', '/materials/education-01.webp', 'https://commons.wikimedia.org/wiki/File:Metal_desk_lamp_and_books_pile_(42381074432).jpg', 'freestocks.org from Olsztyn, Poland', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000020', '20000000-0000-4000-8000-000000000004', '课堂新知', '/materials/education-02.webp', 'https://commons.wikimedia.org/wiki/File:Summer_empty_classroom,_potted_flowers_on_tables,_2023.jpg', 'AKA MBG', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000021', '20000000-0000-4000-8000-000000000004', '毕业季', '/materials/education-03.webp', 'https://commons.wikimedia.org/wiki/File:Unacademic_graduate_HEC_Paris.jpg', 'Jebulon', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000022', '20000000-0000-4000-8000-000000000004', '在线成长', '/materials/education-04.webp', 'https://commons.wikimedia.org/wiki/File:Laptop_on_desk_book_stacks_(Unsplash).jpg', 'freddie marriage fredmarriage', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000023', '20000000-0000-4000-8000-000000000004', '探索科学', '/materials/education-05.webp', 'https://commons.wikimedia.org/wiki/File:Plants_in_beakers_(Unsplash).jpg', 'chuttersnap chuttersnap', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000024', '20000000-0000-4000-8000-000000000004', '知识之境', '/materials/education-06.webp', 'https://commons.wikimedia.org/wiki/File:Estanter%C3%ADa_de_la_Biblioteca_Vasconcelos.jpg', 'Jorge Mendoza', 'CC0 1.0', TRUE, 5),
    ('21000000-0000-4000-8000-000000000025', '20000000-0000-4000-8000-000000000005', '海岛假日', '/materials/travel-01.webp', 'https://commons.wikimedia.org/wiki/File:Parasols,_Evening,_Beach,_Rincon_de_la_Victoria,_Andalusia,_Spain.jpg', 'Jebulon', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000026', '20000000-0000-4000-8000-000000000005', '奔赴山野', '/materials/travel-02.webp', 'https://commons.wikimedia.org/wiki/File:Mountain_landscape_in_kerala.jpg', 'Karthika Manikandan', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000027', '20000000-0000-4000-8000-000000000005', '城市漫游', '/materials/travel-03.webp', 'https://commons.wikimedia.org/wiki/File:Snow-covered_Quebec_City_skyline_in_fog.jpg', 'Wilfredor', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000028', '20000000-0000-4000-8000-000000000005', '公路远行', '/materials/travel-04.webp', 'https://commons.wikimedia.org/wiki/File:Russell%E2%80%93Brasstown_Scenic_Byway_at_Hogpen_Gap,_Oct_2018.jpg', 'Thomson200', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000029', '20000000-0000-4000-8000-000000000005', '古城印象', '/materials/travel-05.webp', 'https://commons.wikimedia.org/wiki/File:Justine-Herbigny_(Ardennes)_vernacular_architecture_Herbigny.JPG', 'Havang(nl)', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000030', '20000000-0000-4000-8000-000000000005', '森林呼吸', '/materials/travel-06.webp', 'https://commons.wikimedia.org/wiki/File:Green_coniferous_forest_(Unsplash).jpg', 'Brownie brownie', 'CC0 1.0', TRUE, 5),
    ('21000000-0000-4000-8000-000000000031', '20000000-0000-4000-8000-000000000006', '元宵喜乐', '/materials/festival-01.webp', 'https://commons.wikimedia.org/wiki/File:Chinese-lanterns-1394958.jpg', 'wiroj from Pixabay', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000032', '20000000-0000-4000-8000-000000000006', '圣诞欢聚', '/materials/festival-02.webp', 'https://commons.wikimedia.org/wiki/File:Christmas_decorations_Helix.jpg', 'さえぼー', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000033', '20000000-0000-4000-8000-000000000006', '新年烟火', '/materials/festival-03.webp', 'https://commons.wikimedia.org/wiki/File:Fireworks_in_Sevierville,_Tennessee_on_4th_of_July_-_15.jpg', 'Roc0ast3r', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000034', '20000000-0000-4000-8000-000000000006', '生日快乐', '/materials/festival-04.webp', 'https://commons.wikimedia.org/wiki/File:Birthday_Cake_Bloomsburg,_Pennsylvania.jpg', 'Tim-desser', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000035', '20000000-0000-4000-8000-000000000006', '花好良缘', '/materials/festival-05.webp', 'https://commons.wikimedia.org/wiki/File:Flowers_for_a_Wedding.jpg', 'Kullatan Kin', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000036', '20000000-0000-4000-8000-000000000006', '丰收之秋', '/materials/festival-06.webp', 'https://commons.wikimedia.org/wiki/File:Pumpkin_Sitting_on_Sauvie_Island.jpg', 'Turtle21680', 'CC0 1.0', TRUE, 5),
    ('21000000-0000-4000-8000-000000000037', '20000000-0000-4000-8000-000000000007', '盛大开业', '/materials/poster-01.webp', 'https://commons.wikimedia.org/wiki/File:Dash_Coffee_at_Gate_1_Dasma_-_Interior_-_West_end_2.jpg', 'UndueMarmot', 'CC0 1.0', TRUE, 0),
    ('21000000-0000-4000-8000-000000000038', '20000000-0000-4000-8000-000000000007', '周年庆典', '/materials/poster-02.webp', 'https://commons.wikimedia.org/wiki/File:A_many_balloon.jpg', 'Knites', 'CC0 1.0', TRUE, 1),
    ('21000000-0000-4000-8000-000000000039', '20000000-0000-4000-8000-000000000007', '诚聘英才', '/materials/poster-03.webp', 'https://commons.wikimedia.org/wiki/File:Chairs_in_a_meeting_room_(Unsplash).jpg', 'Breather breather', 'CC0 1.0', TRUE, 2),
    ('21000000-0000-4000-8000-000000000040', '20000000-0000-4000-8000-000000000007', '艺术展览', '/materials/poster-04.webp', 'https://commons.wikimedia.org/wiki/File:Interior_-_Auckland_Art_Gallery_-_Auckland,_NZ_-_DSC08748.jpg', 'Daderot', 'CC0 1.0', TRUE, 3),
    ('21000000-0000-4000-8000-000000000041', '20000000-0000-4000-8000-000000000007', '音乐现场', '/materials/poster-05.webp', 'https://commons.wikimedia.org/wiki/File:Le_Badaboum_Paris_concert_stage.jpg', 'Shawndeney', 'CC0 1.0', TRUE, 4),
    ('21000000-0000-4000-8000-000000000042', '20000000-0000-4000-8000-000000000007', '温暖公益', '/materials/poster-06.webp', 'https://commons.wikimedia.org/wiki/File:Snow_heart_in_hands_(Unsplash).jpg', 'Brigitte Tohm brigittetohm', 'CC0 1.0', TRUE, 5);
